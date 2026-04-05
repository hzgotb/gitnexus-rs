use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde_json::{Map, Value, json};

use crate::commands::local_kuzu;
use crate::commands::local_mcp::run_server as run_mcp_server;
use crate::commands::local_serve::run_server as run_http_server;
use crate::ingestion::{
    HeuristicCommunity, HeuristicProcess, IngestionResult, StructureNode, StructureRelationship,
    run_ingestion_pipeline,
};
use crate::storage::repo_manager::{find_repo, get_storage_paths, list_registered_repos};

const GRAPH_CACHE_FILENAME: &str = "graph.json";
const DEFAULT_PROCESS_LIMIT: usize = 5;
const DEFAULT_MAX_SYMBOLS_PER_PROCESS: usize = 10;
const DEFAULT_IMPACT_DEPTH: u32 = 3;
const TOOL_REL_TYPES: [&str; 4] = ["CALLS", "IMPORTS", "EXTENDS", "IMPLEMENTS"];

const CYPHER_WRITE_KEYWORDS: [&str; 9] = [
    "CREATE", "DELETE", "SET", "MERGE", "REMOVE", "DROP", "ALTER", "COPY", "DETACH",
];

#[derive(Debug)]
enum Direction {
    Upstream,
    Downstream,
}

#[derive(Debug)]
enum CypherOp {
    Eq,
    Contains,
}

#[derive(Debug)]
struct CypherCond {
    var: String,
    field: String,
    op: CypherOp,
    value: String,
}

#[derive(Debug)]
struct CypherReturnExpr {
    var: String,
    field: String,
    alias: String,
}

#[derive(Debug)]
enum CypherReturnSpec {
    Count { alias: String },
    Columns(Vec<CypherReturnExpr>),
}

#[derive(Debug)]
struct CypherTable {
    headers: Vec<String>,
    rows: Vec<Vec<Value>>,
}

#[derive(Debug)]
struct EdgePattern {
    left_var: String,
    left_label: Option<String>,
    rel_var: String,
    rel_label: Option<String>,
    rel_type_filter: Option<String>,
    right_var: String,
    right_label: Option<String>,
}

#[derive(Debug)]
enum Entity<'a> {
    Node(&'a StructureNode),
    Rel(&'a StructureRelationship),
}

#[derive(Debug)]
struct QueryCandidate {
    score: f64,
    node: StructureNode,
}

#[derive(Debug, Clone)]
struct ProcessMembership {
    process_id: String,
    step_index: u64,
    step_count: u64,
}

#[derive(Debug)]
struct LocalGraph {
    repo_path: PathBuf,
    data: IngestionResult,
    nodes_by_id: HashMap<String, StructureNode>,
    symbol_processes: HashMap<String, Vec<ProcessMembership>>,
    processes_by_id: HashMap<String, HeuristicProcess>,
    community_by_file: HashMap<String, String>,
}

#[derive(Debug)]
struct QueryArgs {
    search_query: String,
    repo: Option<String>,
    context: Option<String>,
    goal: Option<String>,
    limit: usize,
    include_content: bool,
}

#[derive(Debug)]
struct ContextArgs {
    name: Option<String>,
    repo: Option<String>,
    uid: Option<String>,
    file: Option<String>,
    include_content: bool,
}

#[derive(Debug)]
struct ImpactArgs {
    target: String,
    direction: Direction,
    repo: Option<String>,
    depth: u32,
    include_tests: bool,
    relation_types: Vec<String>,
    min_confidence: f32,
}

#[derive(Debug)]
struct CypherArgs {
    query: String,
    repo: Option<String>,
}

#[derive(Debug)]
enum DetectScope {
    Unstaged,
    Staged,
    All,
    Compare,
}

#[derive(Debug)]
struct DetectChangesArgs {
    repo: Option<String>,
    scope: DetectScope,
    base_ref: Option<String>,
}

#[derive(Debug)]
struct RenameArgs {
    symbol_name: Option<String>,
    symbol_uid: Option<String>,
    new_name: String,
    file_path: Option<String>,
    repo: Option<String>,
    dry_run: bool,
}

#[derive(Debug)]
struct ServeArgs {
    port: Option<u16>,
    host: Option<String>,
}

#[derive(Debug, Clone)]
struct RenameEdit {
    line: usize,
    old_text: String,
    new_text: String,
    confidence: String,
}

pub fn try_run_native_tool(subcommand: &str, args: &[String]) -> Result<bool> {
    match subcommand {
        "query" => {
            let parsed = parse_query_args(args)?;
            run_query(parsed)?;
            Ok(true)
        }
        "context" => {
            let parsed = parse_context_args(args)?;
            run_context(parsed)?;
            Ok(true)
        }
        "impact" => {
            let parsed = parse_impact_args(args)?;
            run_impact(parsed)?;
            Ok(true)
        }
        "cypher" => {
            let parsed = parse_cypher_args(args)?;
            run_cypher(parsed)?;
            Ok(true)
        }
        "detect_changes" | "detect-changes" => {
            let parsed = parse_detect_changes_args(args)?;
            run_detect_changes(parsed)?;
            Ok(true)
        }
        "rename" => {
            let parsed = parse_rename_args(args)?;
            run_rename(parsed)?;
            Ok(true)
        }
        "serve" => {
            let parsed = parse_serve_args(args)?;
            run_http_server(parsed.host.as_deref(), parsed.port)?;
            Ok(true)
        }
        "mcp" => {
            run_mcp_server()?;
            Ok(true)
        }
        _ => Ok(false),
    }
}

fn parse_query_args(args: &[String]) -> Result<QueryArgs> {
    if args.is_empty() {
        bail!(
            "Usage: gitnexus query <search_query> [--repo <repo>] [--context <text>] [--goal <goal>] [--limit <n>] [--content]"
        );
    }

    let search_query = args[0].clone();
    let mut repo = None;
    let mut context = None;
    let mut goal = None;
    let mut limit = DEFAULT_PROCESS_LIMIT;
    let mut include_content = false;

    let mut idx = 1usize;
    while idx < args.len() {
        let flag = &args[idx];
        match flag.as_str() {
            "--repo" => {
                repo = Some(next_arg(args, &mut idx, flag)?);
            }
            "--context" => {
                context = Some(next_arg(args, &mut idx, flag)?);
            }
            "--goal" => {
                goal = Some(next_arg(args, &mut idx, flag)?);
            }
            "--limit" => {
                let raw = next_arg(args, &mut idx, flag)?;
                limit = raw
                    .parse::<usize>()
                    .with_context(|| format!("invalid value for --limit: {raw}"))?;
                if limit == 0 {
                    bail!("--limit must be greater than 0");
                }
            }
            "--content" => include_content = true,
            other => bail!("unknown query option: {other}"),
        }
        idx += 1;
    }

    Ok(QueryArgs {
        search_query,
        repo,
        context,
        goal,
        limit,
        include_content,
    })
}

fn parse_context_args(args: &[String]) -> Result<ContextArgs> {
    let mut repo = None;
    let mut uid = None;
    let mut file = None;
    let mut include_content = false;

    let mut idx = 0usize;
    let mut name = None;
    if let Some(first) = args.first()
        && !first.starts_with("--")
    {
        name = Some(first.clone());
        idx = 1;
    }

    while idx < args.len() {
        let flag = &args[idx];
        match flag.as_str() {
            "--repo" => {
                repo = Some(next_arg(args, &mut idx, flag)?);
            }
            "--uid" => {
                uid = Some(next_arg(args, &mut idx, flag)?);
            }
            "--file" => {
                file = Some(next_arg(args, &mut idx, flag)?);
            }
            "--content" => include_content = true,
            other => bail!("unknown context option: {other}"),
        }
        idx += 1;
    }

    if name.is_none() && uid.is_none() {
        bail!(
            "Usage: gitnexus context <symbol_name> [--uid <uid>] [--file <path>] [--repo <repo>] [--content]"
        );
    }

    Ok(ContextArgs {
        name,
        repo,
        uid,
        file,
        include_content,
    })
}

fn parse_impact_args(args: &[String]) -> Result<ImpactArgs> {
    if args.is_empty() {
        bail!(
            "Usage: gitnexus impact <target> [--direction upstream|downstream] [--repo <repo>] [--depth <n>] [--include-tests] [--relation-types <csv>] [--min-confidence <0..1>]"
        );
    }

    let target = args[0].clone();
    let mut direction = Direction::Upstream;
    let mut repo = None;
    let mut depth = DEFAULT_IMPACT_DEPTH;
    let mut include_tests = false;
    let mut relation_type_inputs = Vec::<String>::new();
    let mut min_confidence = 0.0f32;

    let mut idx = 1usize;
    while idx < args.len() {
        let flag = &args[idx];
        match flag.as_str() {
            "--direction" => {
                let raw = next_arg(args, &mut idx, flag)?;
                direction = parse_direction(&raw)?;
            }
            "--repo" => {
                repo = Some(next_arg(args, &mut idx, flag)?);
            }
            "--depth" => {
                let raw = next_arg(args, &mut idx, flag)?;
                depth = raw
                    .parse::<u32>()
                    .with_context(|| format!("invalid value for --depth: {raw}"))?;
                if depth == 0 {
                    bail!("--depth must be greater than 0");
                }
            }
            "--include-tests" => include_tests = true,
            "--relation-types" | "--relation_types" => {
                let raw = next_arg(args, &mut idx, flag)?;
                relation_type_inputs.extend(parse_relation_types(&raw));
            }
            "--min-confidence" | "--min_confidence" => {
                let raw = next_arg(args, &mut idx, flag)?;
                min_confidence = raw
                    .parse::<f32>()
                    .with_context(|| format!("invalid value for --min-confidence: {raw}"))?;
                if !(0.0..=1.0).contains(&min_confidence) {
                    bail!("--min-confidence must be within [0, 1]");
                }
            }
            other => bail!("unknown impact option: {other}"),
        }
        idx += 1;
    }

    let relation_types = normalize_relation_types(relation_type_inputs);

    Ok(ImpactArgs {
        target,
        direction,
        repo,
        depth,
        include_tests,
        relation_types,
        min_confidence,
    })
}

fn parse_relation_types(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
        .map(|item| item.to_ascii_uppercase())
        .collect::<Vec<_>>()
}

fn normalize_relation_types(inputs: Vec<String>) -> Vec<String> {
    let defaults = TOOL_REL_TYPES
        .iter()
        .map(|item| item.to_string())
        .collect::<Vec<_>>();

    if inputs.is_empty() {
        return defaults;
    }

    let mut seen = HashSet::<String>::new();
    let mut filtered = Vec::<String>::new();
    for relation_type in inputs {
        if !TOOL_REL_TYPES.contains(&relation_type.as_str()) {
            continue;
        }
        if seen.insert(relation_type.clone()) {
            filtered.push(relation_type);
        }
    }

    if filtered.is_empty() {
        defaults
    } else {
        filtered
    }
}

fn parse_cypher_args(args: &[String]) -> Result<CypherArgs> {
    if args.is_empty() {
        bail!("Usage: gitnexus cypher <query> [--repo <repo>]");
    }

    let query = args[0].clone();
    let mut repo = None;

    let mut idx = 1usize;
    while idx < args.len() {
        let flag = &args[idx];
        match flag.as_str() {
            "--repo" => {
                repo = Some(next_arg(args, &mut idx, flag)?);
            }
            other => bail!("unknown cypher option: {other}"),
        }
        idx += 1;
    }

    Ok(CypherArgs { query, repo })
}

fn parse_detect_changes_args(args: &[String]) -> Result<DetectChangesArgs> {
    let mut repo = None;
    let mut scope = DetectScope::Unstaged;
    let mut base_ref = None;

    let mut idx = 0usize;
    while idx < args.len() {
        let flag = &args[idx];
        match flag.as_str() {
            "--repo" => {
                repo = Some(next_arg(args, &mut idx, flag)?);
            }
            "--scope" => {
                let raw = next_arg(args, &mut idx, flag)?;
                scope = parse_detect_scope(&raw)?;
            }
            "--base-ref" | "--base_ref" => {
                base_ref = Some(next_arg(args, &mut idx, flag)?);
            }
            other => bail!("unknown detect_changes option: {other}"),
        }
        idx += 1;
    }

    if matches!(scope, DetectScope::Compare) && base_ref.is_none() {
        bail!("--base-ref is required when --scope compare is used");
    }

    Ok(DetectChangesArgs {
        repo,
        scope,
        base_ref,
    })
}

fn parse_rename_args(args: &[String]) -> Result<RenameArgs> {
    let mut symbol_name = None;
    let mut symbol_uid = None;
    let mut new_name = None;
    let mut file_path = None;
    let mut repo = None;
    let mut dry_run = true;

    let mut idx = 0usize;
    while idx < args.len() {
        let flag = &args[idx];
        match flag.as_str() {
            "--symbol-name" => {
                symbol_name = Some(next_arg(args, &mut idx, flag)?);
            }
            "--symbol-uid" => {
                symbol_uid = Some(next_arg(args, &mut idx, flag)?);
            }
            "--new-name" => {
                new_name = Some(next_arg(args, &mut idx, flag)?);
            }
            "--file-path" | "--file" => {
                file_path = Some(next_arg(args, &mut idx, flag)?);
            }
            "--repo" => {
                repo = Some(next_arg(args, &mut idx, flag)?);
            }
            "--dry-run" => {
                let raw = next_arg(args, &mut idx, flag)?;
                dry_run = parse_bool_flag(&raw)?;
            }
            "--apply" => {
                dry_run = false;
            }
            other => bail!("unknown rename option: {other}"),
        }
        idx += 1;
    }

    let Some(new_name) = new_name else {
        bail!("rename requires --new-name <name>");
    };
    if symbol_name.is_none() && symbol_uid.is_none() {
        bail!("rename requires either --symbol-name <name> or --symbol-uid <uid>");
    }
    if !is_valid_ident(&new_name) {
        bail!(
            "invalid --new-name: {new_name} (must be identifier-like: letters/digits/underscore)"
        );
    }

    Ok(RenameArgs {
        symbol_name,
        symbol_uid,
        new_name,
        file_path,
        repo,
        dry_run,
    })
}

fn parse_serve_args(args: &[String]) -> Result<ServeArgs> {
    let mut port = None;
    let mut host = None;

    let mut idx = 0usize;
    while idx < args.len() {
        let flag = &args[idx];
        match flag.as_str() {
            "--port" => {
                let raw = next_arg(args, &mut idx, flag)?;
                let value = raw
                    .parse::<u16>()
                    .with_context(|| format!("invalid value for --port: {raw}"))?;
                port = Some(value);
            }
            "--host" => {
                host = Some(next_arg(args, &mut idx, flag)?);
            }
            other => bail!("unknown serve option: {other}"),
        }
        idx += 1;
    }

    Ok(ServeArgs { port, host })
}

fn next_arg(args: &[String], idx: &mut usize, flag: &str) -> Result<String> {
    let next = *idx + 1;
    let Some(value) = args.get(next) else {
        bail!("missing value for {flag}");
    };
    *idx = next;
    Ok(value.clone())
}

fn parse_direction(raw: &str) -> Result<Direction> {
    match raw.to_ascii_lowercase().as_str() {
        "upstream" => Ok(Direction::Upstream),
        "downstream" => Ok(Direction::Downstream),
        _ => bail!("invalid --direction: {raw} (expected upstream|downstream)"),
    }
}

fn parse_detect_scope(raw: &str) -> Result<DetectScope> {
    match raw.to_ascii_lowercase().as_str() {
        "unstaged" => Ok(DetectScope::Unstaged),
        "staged" => Ok(DetectScope::Staged),
        "all" => Ok(DetectScope::All),
        "compare" => Ok(DetectScope::Compare),
        _ => bail!("invalid --scope: {raw} (expected unstaged|staged|all|compare)"),
    }
}

fn parse_bool_flag(raw: &str) -> Result<bool> {
    match raw.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "y" => Ok(true),
        "0" | "false" | "no" | "n" => Ok(false),
        _ => bail!("invalid boolean value: {raw}"),
    }
}

fn run_query(args: QueryArgs) -> Result<()> {
    match local_kuzu::try_run_query(
        &args.search_query,
        args.repo.as_deref(),
        args.context.as_deref(),
        args.goal.as_deref(),
        args.limit,
        args.include_content,
    ) {
        Ok(Some(payload)) => {
            println!("{}", serde_json::to_string_pretty(&payload)?);
            return Ok(());
        }
        Ok(None) => {}
        Err(err) => {
            eprintln!(
                "GitNexus: native Kuzu query bridge failed, falling back to heuristic engine: {err}"
            );
        }
    }

    let graph = load_graph(args.repo.as_deref())?;
    let term_text = format!(
        "{} {} {}",
        args.search_query,
        args.context.as_deref().unwrap_or_default(),
        args.goal.as_deref().unwrap_or_default()
    );
    let terms = tokenize(&term_text);

    if terms.is_empty() {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "processes": [],
                "process_symbols": [],
                "definitions": []
            }))?
        );
        return Ok(());
    }

    let mut candidates = Vec::<QueryCandidate>::new();
    for node in graph
        .data
        .graph
        .nodes
        .iter()
        .filter(|n| is_symbol_label(&n.label))
    {
        let score = score_node(node, &terms);
        if score <= 0.0 {
            continue;
        }
        candidates.push(QueryCandidate {
            score,
            node: node.clone(),
        });
    }

    candidates.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal));
    let search_limit = args.limit * DEFAULT_MAX_SYMBOLS_PER_PROCESS * 3;
    let capped = candidates.into_iter().take(search_limit);

    #[derive(Default)]
    struct ProcessBucket {
        total_score: f64,
        symbols: Vec<Value>,
    }

    let mut process_buckets = HashMap::<String, ProcessBucket>::new();
    let mut definitions = Vec::<Value>::new();

    for candidate in capped {
        let memberships = graph.symbol_processes.get(&candidate.node.id);
        let module = graph
            .community_by_file
            .get(&candidate.node.file_path)
            .cloned()
            .unwrap_or_default();
        let content = maybe_symbol_content(
            &graph.repo_path,
            &candidate.node.file_path,
            candidate.node.start_line,
            args.include_content,
        );

        let mut base = Map::<String, Value>::new();
        base.insert("id".to_string(), Value::String(candidate.node.id.clone()));
        base.insert(
            "name".to_string(),
            Value::String(candidate.node.name.clone()),
        );
        base.insert(
            "type".to_string(),
            Value::String(candidate.node.label.clone()),
        );
        base.insert(
            "filePath".to_string(),
            Value::String(candidate.node.file_path.clone()),
        );
        if let Some(start) = candidate.node.start_line {
            base.insert("startLine".to_string(), Value::Number(start.into()));
            base.insert("endLine".to_string(), Value::Number(start.into()));
        }
        if !module.is_empty() {
            base.insert("module".to_string(), Value::String(module));
        }
        if let Some(content) = content {
            base.insert("content".to_string(), Value::String(content));
        }

        if let Some(memberships) = memberships {
            for membership in memberships {
                let mut entry = base.clone();
                entry.insert(
                    "process_id".to_string(),
                    Value::String(membership.process_id.clone()),
                );
                entry.insert(
                    "step_index".to_string(),
                    Value::Number(membership.step_index.into()),
                );

                let bucket = process_buckets
                    .entry(membership.process_id.clone())
                    .or_default();
                bucket.total_score += candidate.score;
                bucket.symbols.push(Value::Object(entry));
            }
        } else {
            definitions.push(Value::Object(base));
        }
    }

    #[derive(Debug)]
    struct RankedProcess {
        id: String,
        summary: String,
        priority: f64,
        step_count: u64,
        symbols: Vec<Value>,
    }

    let mut ranked = Vec::<RankedProcess>::new();
    for (process_id, bucket) in process_buckets {
        let Some(process) = graph.processes_by_id.get(&process_id) else {
            continue;
        };

        let summary = process_summary(process, &graph.nodes_by_id);
        ranked.push(RankedProcess {
            id: process.id.clone(),
            summary,
            priority: bucket.total_score,
            step_count: process.step_count,
            symbols: bucket.symbols,
        });
    }

    ranked.sort_by(|a, b| {
        b.priority
            .partial_cmp(&a.priority)
            .unwrap_or(Ordering::Equal)
    });

    let selected_processes = ranked.into_iter().take(args.limit).collect::<Vec<_>>();
    let processes = selected_processes
        .iter()
        .map(|p| {
            json!({
                "id": p.id,
                "summary": p.summary,
                "priority": round3(p.priority),
                "symbol_count": p.symbols.len(),
                "process_type": "heuristic",
                "step_count": p.step_count
            })
        })
        .collect::<Vec<_>>();

    let mut seen = HashSet::<String>::new();
    let mut process_symbols = Vec::<Value>::new();
    for process in &selected_processes {
        for symbol in process.symbols.iter().take(DEFAULT_MAX_SYMBOLS_PER_PROCESS) {
            let Some(id) = symbol.get("id").and_then(Value::as_str) else {
                continue;
            };
            if !seen.insert(id.to_string()) {
                continue;
            }
            process_symbols.push(symbol.clone());
        }
    }

    let output = json!({
        "processes": processes,
        "process_symbols": process_symbols,
        "definitions": definitions.into_iter().take(20).collect::<Vec<_>>()
    });

    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

fn run_context(args: ContextArgs) -> Result<()> {
    match local_kuzu::try_run_context(
        args.name.as_deref(),
        args.uid.as_deref(),
        args.file.as_deref(),
        args.repo.as_deref(),
        args.include_content,
    ) {
        Ok(Some(payload)) => {
            println!("{}", serde_json::to_string_pretty(&payload)?);
            return Ok(());
        }
        Ok(None) => {}
        Err(err) => {
            eprintln!(
                "GitNexus: native Kuzu context bridge failed, falling back to heuristic engine: {err}"
            );
        }
    }

    let graph = load_graph(args.repo.as_deref())?;

    let mut matches = if let Some(uid) = args.uid.as_deref() {
        graph
            .data
            .graph
            .nodes
            .iter()
            .filter(|n| n.id == uid)
            .cloned()
            .collect::<Vec<_>>()
    } else {
        let Some(name) = args.name.as_deref() else {
            bail!("context requires either name or --uid");
        };

        graph
            .data
            .graph
            .nodes
            .iter()
            .filter(|n| is_symbol_label(&n.label) && (n.name == name || n.id == name))
            .cloned()
            .collect::<Vec<_>>()
    };

    if let Some(file_filter) = args.file.as_deref() {
        matches.retain(|n| n.file_path.contains(file_filter));
    }

    if matches.is_empty() {
        let needle = args
            .uid
            .as_deref()
            .or(args.name.as_deref())
            .unwrap_or("unknown");
        println!(
            "{}",
            serde_json::to_string_pretty(
                &json!({ "error": format!("Symbol '{needle}' not found") })
            )?
        );
        return Ok(());
    }

    if matches.len() > 1 && args.uid.is_none() {
        let candidates = matches
            .iter()
            .take(10)
            .map(|n| {
                json!({
                    "uid": n.id,
                    "name": n.name,
                    "kind": n.label,
                    "filePath": n.file_path,
                    "line": n.start_line
                })
            })
            .collect::<Vec<_>>();

        let output = json!({
            "status": "ambiguous",
            "message": format!("Found {} symbols matching '{}'. Use --uid or --file to disambiguate.", matches.len(), args.name.unwrap_or_default()),
            "candidates": candidates
        });
        println!("{}", serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    let sym = matches.into_iter().next().unwrap_or_else(|| unreachable!());
    let mut incoming = BTreeMap::<String, Vec<Value>>::new();
    let mut outgoing = BTreeMap::<String, Vec<Value>>::new();

    for rel in graph
        .data
        .graph
        .relationships
        .iter()
        .filter(|rel| TOOL_REL_TYPES.contains(&rel.rel_type.as_str()))
    {
        if rel.target_id == sym.id
            && let Some(src) = graph.nodes_by_id.get(&rel.source_id)
        {
            incoming
                .entry(rel.rel_type.to_ascii_lowercase())
                .or_default()
                .push(node_ref(src));
        }

        if rel.source_id == sym.id
            && let Some(dst) = graph.nodes_by_id.get(&rel.target_id)
        {
            outgoing
                .entry(rel.rel_type.to_ascii_lowercase())
                .or_default()
                .push(node_ref(dst));
        }
    }

    for refs in incoming.values_mut() {
        dedupe_refs(refs);
        refs.truncate(30);
    }
    for refs in outgoing.values_mut() {
        dedupe_refs(refs);
        refs.truncate(30);
    }

    let processes = graph
        .symbol_processes
        .get(&sym.id)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|membership| {
            let process = graph.processes_by_id.get(&membership.process_id)?;
            Some(json!({
                "id": process.id,
                "name": process_summary(process, &graph.nodes_by_id),
                "step_index": membership.step_index,
                "step_count": membership.step_count,
            }))
        })
        .collect::<Vec<_>>();

    let mut symbol_obj = Map::<String, Value>::new();
    symbol_obj.insert("uid".to_string(), Value::String(sym.id.clone()));
    symbol_obj.insert("name".to_string(), Value::String(sym.name.clone()));
    symbol_obj.insert("kind".to_string(), Value::String(sym.label.clone()));
    symbol_obj.insert("filePath".to_string(), Value::String(sym.file_path.clone()));
    if let Some(line) = sym.start_line {
        symbol_obj.insert("startLine".to_string(), Value::Number(line.into()));
        symbol_obj.insert("endLine".to_string(), Value::Number(line.into()));
    }

    if let Some(content) = maybe_symbol_content(
        &graph.repo_path,
        &sym.file_path,
        sym.start_line,
        args.include_content,
    ) {
        symbol_obj.insert("content".to_string(), Value::String(content));
    }

    let output = json!({
        "status": "found",
        "symbol": symbol_obj,
        "incoming": incoming,
        "outgoing": outgoing,
        "processes": processes,
    });

    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

fn run_impact(args: ImpactArgs) -> Result<()> {
    let direction = match &args.direction {
        Direction::Upstream => "upstream",
        Direction::Downstream => "downstream",
    };
    match local_kuzu::try_run_impact(
        &args.target,
        direction,
        args.repo.as_deref(),
        args.depth,
        args.include_tests,
        &args.relation_types,
        args.min_confidence,
    ) {
        Ok(Some(payload)) => {
            println!("{}", serde_json::to_string_pretty(&payload)?);
            return Ok(());
        }
        Ok(None) => {}
        Err(err) => {
            eprintln!(
                "GitNexus: native Kuzu impact bridge failed, falling back to heuristic engine: {err}"
            );
        }
    }

    let graph = load_graph(args.repo.as_deref())?;

    let mut targets = graph
        .data
        .graph
        .nodes
        .iter()
        .filter(|n| is_symbol_label(&n.label) && n.name == args.target)
        .cloned()
        .collect::<Vec<_>>();

    targets.sort_by(|a, b| a.file_path.cmp(&b.file_path));

    let Some(target) = targets.first() else {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "error": format!("Target '{}' not found", args.target)
            }))?
        );
        return Ok(());
    };

    let mut visited = HashSet::<String>::new();
    visited.insert(target.id.clone());

    #[derive(Debug, Clone)]
    struct ImpactItem {
        depth: u32,
        id: String,
        name: String,
        node_type: String,
        file_path: String,
        relation_type: String,
        confidence: f32,
    }

    let mut impacted = Vec::<ImpactItem>::new();
    let mut frontier = vec![target.id.clone()];
    let relation_type_filter = args
        .relation_types
        .iter()
        .map(String::as_str)
        .collect::<HashSet<_>>();

    for depth in 1..=args.depth {
        if frontier.is_empty() {
            break;
        }

        let frontier_set = frontier.iter().cloned().collect::<HashSet<_>>();
        let mut next_frontier = Vec::<String>::new();

        for rel in graph
            .data
            .graph
            .relationships
            .iter()
            .filter(|rel| relation_type_filter.contains(rel.rel_type.as_str()))
            .filter(|rel| rel.confidence >= args.min_confidence)
        {
            let candidate = match args.direction {
                Direction::Upstream if frontier_set.contains(&rel.target_id) => {
                    Some((&rel.source_id, rel.rel_type.clone(), rel.confidence))
                }
                Direction::Downstream if frontier_set.contains(&rel.source_id) => {
                    Some((&rel.target_id, rel.rel_type.clone(), rel.confidence))
                }
                _ => None,
            };

            let Some((node_id, relation_type, confidence)) = candidate else {
                continue;
            };

            if visited.contains(node_id) {
                continue;
            }

            let Some(node) = graph.nodes_by_id.get(node_id) else {
                continue;
            };
            if !is_symbol_label(&node.label) {
                continue;
            }
            if !args.include_tests && is_test_file_path(&node.file_path) {
                continue;
            }

            visited.insert(node_id.clone());
            next_frontier.push(node_id.clone());
            impacted.push(ImpactItem {
                depth,
                id: node.id.clone(),
                name: node.name.clone(),
                node_type: node.label.clone(),
                file_path: node.file_path.clone(),
                relation_type,
                confidence,
            });
        }

        frontier = next_frontier;
    }

    let mut by_depth = BTreeMap::<u32, Vec<Value>>::new();
    for item in &impacted {
        by_depth.entry(item.depth).or_default().push(json!({
            "depth": item.depth,
            "id": item.id,
            "name": item.name,
            "type": item.node_type,
            "filePath": item.file_path,
            "relationType": item.relation_type,
            "confidence": item.confidence,
        }));
    }

    let impacted_ids = impacted
        .iter()
        .map(|i| i.id.clone())
        .collect::<HashSet<String>>();
    let direct_ids = impacted
        .iter()
        .filter(|i| i.depth == 1)
        .map(|i| i.id.clone())
        .collect::<HashSet<_>>();

    let mut process_hits = HashMap::<String, (u64, u64, u64)>::new();
    for symbol_id in &impacted_ids {
        if let Some(memberships) = graph.symbol_processes.get(symbol_id) {
            for membership in memberships {
                let entry = process_hits
                    .entry(membership.process_id.clone())
                    .or_insert((0, membership.step_index, membership.step_count));
                entry.0 += 1;
                entry.1 = entry.1.min(membership.step_index);
            }
        }
    }

    let mut affected_processes = process_hits
        .into_iter()
        .filter_map(|(process_id, (hits, min_step, step_count))| {
            let process = graph.processes_by_id.get(&process_id)?;
            Some(json!({
                "name": process_summary(process, &graph.nodes_by_id),
                "hits": hits,
                "broken_at_step": min_step,
                "step_count": step_count,
            }))
        })
        .collect::<Vec<_>>();
    affected_processes.sort_by(|a, b| b["hits"].as_u64().cmp(&a["hits"].as_u64()));

    let mut module_hits = HashMap::<String, u64>::new();
    let mut direct_modules = HashSet::<String>::new();

    for item in &impacted {
        let Some(module) = graph.community_by_file.get(&item.file_path) else {
            continue;
        };

        *module_hits.entry(module.clone()).or_insert(0) += 1;
        if direct_ids.contains(&item.id) {
            direct_modules.insert(module.clone());
        }
    }

    let mut affected_modules = module_hits
        .into_iter()
        .map(|(module, hits)| {
            json!({
                "name": module,
                "hits": hits,
                "impact": if direct_modules.contains(&module) { "direct" } else { "indirect" }
            })
        })
        .collect::<Vec<_>>();
    affected_modules.sort_by(|a, b| b["hits"].as_u64().cmp(&a["hits"].as_u64()));

    let direct_count = by_depth.get(&1).map_or(0, Vec::len);
    let process_count = affected_processes.len();
    let module_count = affected_modules.len();
    let total_count = impacted.len();

    let risk = if direct_count >= 30
        || process_count >= 5
        || module_count >= 5
        || total_count >= 200
    {
        "CRITICAL"
    } else if direct_count >= 15 || process_count >= 3 || module_count >= 3 || total_count >= 100 {
        "HIGH"
    } else if direct_count >= 5 || total_count >= 30 {
        "MEDIUM"
    } else {
        "LOW"
    };

    let direction = match args.direction {
        Direction::Upstream => "upstream",
        Direction::Downstream => "downstream",
    };

    let output = json!({
        "target": {
            "id": target.id,
            "name": target.name,
            "type": target.label,
            "filePath": target.file_path,
        },
        "direction": direction,
        "impactedCount": total_count,
        "risk": risk,
        "summary": {
            "direct": direct_count,
            "processes_affected": process_count,
            "modules_affected": module_count,
        },
        "affected_processes": affected_processes,
        "affected_modules": affected_modules,
        "byDepth": by_depth,
    });

    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

fn run_cypher(args: CypherArgs) -> Result<()> {
    if is_write_cypher(&args.query) {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "error": "Write operations are not allowed. The graph is read-only."
            }))?
        );
        return Ok(());
    }

    if let Some(payload) = local_kuzu::try_run_cypher(&args.query, args.repo.as_deref())? {
        println!("{}", serde_json::to_string_pretty(&payload)?);
        return Ok(());
    }

    let graph = load_graph(args.repo.as_deref())?;
    let table = evaluate_cypher(&graph, &args.query)?;
    let markdown = render_markdown_table(&table);
    let output = json!({
        "markdown": markdown,
        "row_count": table.rows.len(),
    });

    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

fn run_detect_changes(args: DetectChangesArgs) -> Result<()> {
    let graph = load_graph(args.repo.as_deref())?;

    let mut git_args = vec!["diff", "--name-only"];
    match args.scope {
        DetectScope::Unstaged => {}
        DetectScope::Staged => git_args = vec!["diff", "--staged", "--name-only"],
        DetectScope::All => git_args = vec!["diff", "HEAD", "--name-only"],
        DetectScope::Compare => {
            let Some(base_ref) = args.base_ref.as_deref() else {
                bail!("--base-ref is required when --scope compare is used");
            };
            git_args = vec!["diff", base_ref, "--name-only"];
        }
    }

    let output = std::process::Command::new("git")
        .args(&git_args)
        .current_dir(&graph.repo_path)
        .output();

    let Ok(output) = output else {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "error": "Git diff failed: unable to execute git command"
            }))?
        );
        return Ok(());
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let detail = if stderr.is_empty() {
            "unknown error".to_string()
        } else {
            stderr
        };
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "error": format!("Git diff failed: {detail}")
            }))?
        );
        return Ok(());
    }

    let changed_files = String::from_utf8_lossy(&output.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| line.replace('\\', "/"))
        .collect::<Vec<_>>();

    if changed_files.is_empty() {
        let no_changes = json!({
            "summary": {
                "changed_count": 0,
                "affected_count": 0,
                "risk_level": "none",
                "message": "No changes detected."
            },
            "changed_symbols": [],
            "affected_processes": []
        });
        println!("{}", serde_json::to_string_pretty(&no_changes)?);
        return Ok(());
    }

    #[derive(Debug)]
    struct ChangedSymbol {
        id: String,
        name: String,
        node_type: String,
        file_path: String,
    }

    let mut changed_symbols = Vec::<ChangedSymbol>::new();
    let mut seen_symbol_ids = HashSet::<String>::new();
    for node in graph
        .data
        .graph
        .nodes
        .iter()
        .filter(|n| is_symbol_label(&n.label))
    {
        if !changed_files
            .iter()
            .any(|file| path_matches_changed_file(&node.file_path, file))
        {
            continue;
        }

        if !seen_symbol_ids.insert(node.id.clone()) {
            continue;
        }

        changed_symbols.push(ChangedSymbol {
            id: node.id.clone(),
            name: node.name.clone(),
            node_type: node.label.clone(),
            file_path: node.file_path.clone(),
        });
    }

    let changed_symbols_json = changed_symbols
        .iter()
        .map(|sym| {
            json!({
                "id": sym.id,
                "name": sym.name,
                "type": sym.node_type,
                "filePath": sym.file_path,
                "change_type": "Modified",
            })
        })
        .collect::<Vec<_>>();

    #[derive(Debug)]
    struct ChangedStep {
        step: u64,
        symbol: String,
    }

    #[derive(Debug)]
    struct AffectedProcess {
        id: String,
        name: String,
        step_count: u64,
        changed_steps: Vec<ChangedStep>,
    }

    let mut affected_map = HashMap::<String, AffectedProcess>::new();
    for symbol in &changed_symbols {
        if let Some(memberships) = graph.symbol_processes.get(&symbol.id) {
            for membership in memberships {
                let Some(process) = graph.processes_by_id.get(&membership.process_id) else {
                    continue;
                };

                let entry =
                    affected_map
                        .entry(process.id.clone())
                        .or_insert_with(|| AffectedProcess {
                            id: process.id.clone(),
                            name: process_summary(process, &graph.nodes_by_id),
                            step_count: process.step_count,
                            changed_steps: Vec::new(),
                        });

                entry.changed_steps.push(ChangedStep {
                    step: membership.step_index,
                    symbol: symbol.name.clone(),
                });
            }
        }
    }

    let mut affected_processes = affected_map.into_values().collect::<Vec<_>>();
    affected_processes.sort_by(|a, b| b.changed_steps.len().cmp(&a.changed_steps.len()));
    for process in &mut affected_processes {
        process
            .changed_steps
            .sort_by(|a, b| a.step.cmp(&b.step).then(a.symbol.cmp(&b.symbol)));
        process
            .changed_steps
            .dedup_by(|a, b| a.step == b.step && a.symbol == b.symbol);
    }

    let affected_processes_json = affected_processes
        .iter()
        .map(|proc| {
            json!({
                "id": proc.id,
                "name": proc.name,
                "process_type": "heuristic",
                "step_count": proc.step_count,
                "changed_steps": proc.changed_steps.iter().map(|s| json!({
                    "symbol": s.symbol,
                    "step": s.step
                })).collect::<Vec<_>>(),
            })
        })
        .collect::<Vec<_>>();

    let process_count = affected_processes_json.len();
    let risk = if process_count == 0 {
        "low"
    } else if process_count <= 5 {
        "medium"
    } else if process_count <= 15 {
        "high"
    } else {
        "critical"
    };

    let result = json!({
        "summary": {
            "changed_count": changed_symbols_json.len(),
            "affected_count": process_count,
            "changed_files": changed_files.len(),
            "risk_level": risk,
        },
        "changed_symbols": changed_symbols_json,
        "affected_processes": affected_processes_json,
    });

    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

fn run_rename(args: RenameArgs) -> Result<()> {
    let graph = load_graph(args.repo.as_deref())?;

    let mut candidates = if let Some(uid) = args.symbol_uid.as_deref() {
        graph
            .data
            .graph
            .nodes
            .iter()
            .filter(|n| is_symbol_label(&n.label) && n.id == uid)
            .cloned()
            .collect::<Vec<_>>()
    } else {
        let sym_name = args.symbol_name.as_deref().unwrap_or_default();
        graph
            .data
            .graph
            .nodes
            .iter()
            .filter(|n| is_symbol_label(&n.label) && n.name == sym_name)
            .cloned()
            .collect::<Vec<_>>()
    };

    if let Some(file_path) = args.file_path.as_deref() {
        candidates.retain(|n| n.file_path.contains(file_path));
    }

    if candidates.is_empty() {
        let lookup = args
            .symbol_uid
            .as_deref()
            .or(args.symbol_name.as_deref())
            .unwrap_or("unknown");
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "error": format!("Symbol '{lookup}' not found")
            }))?
        );
        return Ok(());
    }

    if candidates.len() > 1 && args.symbol_uid.is_none() {
        let name = args.symbol_name.as_deref().unwrap_or_default();
        let ambiguous = json!({
            "status": "ambiguous",
            "message": format!("Found {} symbols matching '{}'. Use --symbol-uid or --file-path to disambiguate.", candidates.len(), name),
            "candidates": candidates.iter().take(10).map(|n| json!({
                "uid": n.id,
                "name": n.name,
                "kind": n.label,
                "filePath": n.file_path,
                "line": n.start_line,
            })).collect::<Vec<_>>()
        });
        println!("{}", serde_json::to_string_pretty(&ambiguous)?);
        return Ok(());
    }

    let symbol = candidates
        .into_iter()
        .next()
        .unwrap_or_else(|| unreachable!());
    let old_name = symbol.name.clone();
    if old_name == args.new_name {
        println!(
            "{}",
            serde_json::to_string_pretty(&json!({
                "error": "New name is the same as the current name."
            }))?
        );
        return Ok(());
    }

    let mut changes = BTreeMap::<String, Vec<RenameEdit>>::new();
    let mut graph_edits = 0usize;
    let mut text_search_edits = 0usize;

    let mut graph_files = HashSet::<String>::new();
    graph_files.insert(symbol.file_path.clone());
    for rel in graph
        .data
        .graph
        .relationships
        .iter()
        .filter(|r| TOOL_REL_TYPES.contains(&r.rel_type.as_str()) && r.target_id == symbol.id)
    {
        if let Some(src) = graph.nodes_by_id.get(&rel.source_id) {
            graph_files.insert(src.file_path.clone());
        }
    }

    for file in &graph_files {
        if let Some(edits) =
            collect_file_edits(&graph.repo_path, file, &old_name, &args.new_name, "graph")?
        {
            graph_edits += edits.len();
            changes.entry(file.clone()).or_default().extend(edits);
        }
    }

    for scanned in &graph.data.scanned_files {
        if graph_files.contains(&scanned.path) {
            continue;
        }
        if let Some(edits) = collect_file_edits(
            &graph.repo_path,
            &scanned.path,
            &old_name,
            &args.new_name,
            "text_search",
        )? {
            text_search_edits += edits.len();
            changes
                .entry(scanned.path.clone())
                .or_default()
                .extend(edits);
        }
    }

    let mut change_items = changes
        .into_iter()
        .map(|(file_path, mut edits)| {
            edits.sort_by(|a, b| {
                a.line
                    .cmp(&b.line)
                    .then(a.confidence.cmp(&b.confidence))
                    .then(a.old_text.cmp(&b.old_text))
            });
            edits.dedup_by(|a, b| {
                a.line == b.line
                    && a.old_text == b.old_text
                    && a.new_text == b.new_text
                    && a.confidence == b.confidence
            });
            (file_path, edits)
        })
        .collect::<Vec<_>>();
    change_items.sort_by(|a, b| a.0.cmp(&b.0));

    let total_edits = change_items
        .iter()
        .map(|(_, edits)| edits.len())
        .sum::<usize>();
    let files_affected = change_items.len();

    if !args.dry_run {
        for (file_path, _) in &change_items {
            let abs = graph.repo_path.join(file_path);
            let raw = match std::fs::read_to_string(&abs) {
                Ok(raw) => raw,
                Err(_) => continue,
            };

            let (updated, replaced) =
                replace_identifier_occurrences(&raw, &old_name, &args.new_name);
            if replaced == 0 {
                continue;
            }
            std::fs::write(&abs, updated)
                .with_context(|| format!("failed to write {}", abs.to_string_lossy()))?;
        }
    }

    let output = json!({
        "status": "success",
        "old_name": old_name,
        "new_name": args.new_name,
        "files_affected": files_affected,
        "total_edits": total_edits,
        "graph_edits": graph_edits,
        "text_search_edits": text_search_edits,
        "changes": change_items.into_iter().map(|(file_path, edits)| json!({
            "file_path": file_path,
            "edits": edits.into_iter().map(|e| json!({
                "line": e.line,
                "old_text": e.old_text,
                "new_text": e.new_text,
                "confidence": e.confidence
            })).collect::<Vec<_>>()
        })).collect::<Vec<_>>(),
        "applied": !args.dry_run
    });
    println!("{}", serde_json::to_string_pretty(&output)?);
    Ok(())
}

fn load_graph(repo_hint: Option<&str>) -> Result<LocalGraph> {
    let repo_path = resolve_repo_path(repo_hint)?;
    let storage = get_storage_paths(&repo_path);
    let graph_path = storage.storage_path.join(GRAPH_CACHE_FILENAME);

    let data = if graph_path.exists() {
        let raw = std::fs::read_to_string(&graph_path)
            .with_context(|| format!("failed to read {}", graph_path.to_string_lossy()))?;
        serde_json::from_str::<IngestionResult>(&raw)
            .with_context(|| format!("failed to parse {}", graph_path.to_string_lossy()))?
    } else {
        let result = run_ingestion_pipeline(&repo_path)?;
        std::fs::create_dir_all(&storage.storage_path).with_context(|| {
            format!(
                "failed to create storage directory {}",
                storage.storage_path.to_string_lossy()
            )
        })?;
        let content = serde_json::to_string_pretty(&result)?;
        std::fs::write(&graph_path, content)
            .with_context(|| format!("failed to write {}", graph_path.to_string_lossy()))?;
        result
    };

    let nodes_by_id = data
        .graph
        .nodes
        .iter()
        .cloned()
        .map(|n| (n.id.clone(), n))
        .collect::<HashMap<_, _>>();

    let symbol_processes = build_symbol_process_map(&data.processes);
    let processes_by_id = data
        .processes
        .iter()
        .cloned()
        .map(|p| (p.id.clone(), p))
        .collect::<HashMap<_, _>>();
    let community_by_file = build_file_community_map(&data.communities);

    Ok(LocalGraph {
        repo_path,
        data,
        nodes_by_id,
        symbol_processes,
        processes_by_id,
        community_by_file,
    })
}

fn resolve_repo_path(repo_hint: Option<&str>) -> Result<PathBuf> {
    if let Some(hint) = repo_hint {
        return resolve_repo_by_hint(hint);
    }

    let cwd = std::env::current_dir().context("failed to read current working directory")?;
    if let Some(indexed) = find_repo(&cwd)? {
        return Ok(indexed.repo_path);
    }

    let entries = list_registered_repos(true)?;
    match entries.len() {
        0 => bail!("No indexed repositories found. Run: gitnexus analyze"),
        1 => Ok(PathBuf::from(&entries[0].path)),
        _ => {
            let names = entries.iter().map(|e| e.name.clone()).collect::<Vec<_>>();
            bail!(
                "Multiple repositories indexed. Use --repo to disambiguate. Available: {}",
                names.join(", ")
            )
        }
    }
}

fn resolve_repo_by_hint(hint: &str) -> Result<PathBuf> {
    let entries = list_registered_repos(true)?;
    if entries.is_empty() {
        bail!("No indexed repositories found. Run: gitnexus analyze");
    }

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
        .collect::<Vec<_>>();

    matches.sort_by(|a, b| a.name.cmp(&b.name));
    matches.dedup_by(|a, b| a.path == b.path);

    match matches.len() {
        0 => bail!("Repository '{hint}' not found. Run `gitnexus list` for available repos."),
        1 => Ok(PathBuf::from(&matches[0].path)),
        _ => {
            let names = matches.iter().map(|e| e.name.clone()).collect::<Vec<_>>();
            bail!(
                "Repository '{hint}' is ambiguous. Candidates: {}",
                names.join(", ")
            )
        }
    }
}

fn build_symbol_process_map(
    processes: &[HeuristicProcess],
) -> HashMap<String, Vec<ProcessMembership>> {
    let mut map = HashMap::<String, Vec<ProcessMembership>>::new();
    for process in processes {
        for (index, symbol_id) in process.symbols.iter().enumerate() {
            map.entry(symbol_id.clone())
                .or_default()
                .push(ProcessMembership {
                    process_id: process.id.clone(),
                    step_index: (index as u64) + 1,
                    step_count: process.step_count,
                });
        }
    }
    map
}

fn build_file_community_map(communities: &[HeuristicCommunity]) -> HashMap<String, String> {
    let mut map = HashMap::<String, String>::new();
    for community in communities {
        for file in &community.files {
            map.insert(file.clone(), community.id.clone());
        }
    }
    map
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

fn tokenize(text: &str) -> Vec<String> {
    text.split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_' && ch != '$' && ch != '/')
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| s.len() >= 2)
        .collect::<Vec<_>>()
}

fn score_node(node: &StructureNode, terms: &[String]) -> f64 {
    let name = node.name.to_ascii_lowercase();
    let file = node.file_path.to_ascii_lowercase();
    let label = node.label.to_ascii_lowercase();

    let mut score = 0.0;
    for term in terms {
        if name == *term {
            score += 12.0;
            continue;
        }
        if name.contains(term) {
            score += 6.0;
        }
        if file.contains(term) {
            score += 2.0;
        }
        if label.contains(term) {
            score += 1.0;
        }
    }

    score
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

fn maybe_symbol_content(
    repo_path: &Path,
    rel_file_path: &str,
    start_line: Option<u64>,
    include_content: bool,
) -> Option<String> {
    if !include_content {
        return None;
    }

    let abs_path = repo_path.join(rel_file_path);
    let raw = std::fs::read_to_string(abs_path).ok()?;
    let lines = raw.lines().collect::<Vec<_>>();
    if lines.is_empty() {
        return None;
    }

    let start = start_line.unwrap_or(1).saturating_sub(3) as usize;
    let start = start.min(lines.len().saturating_sub(1));
    let end = (start + 25).min(lines.len());

    let snippet = lines[start..end].join("\n");
    if snippet.is_empty() {
        None
    } else {
        Some(snippet)
    }
}

fn node_ref(node: &StructureNode) -> Value {
    json!({
        "uid": node.id,
        "name": node.name,
        "filePath": node.file_path,
        "kind": node.label,
    })
}

fn dedupe_refs(refs: &mut Vec<Value>) {
    let mut seen = HashSet::<String>::new();
    refs.retain(|r| {
        let key = format!(
            "{}::{}::{}",
            r.get("uid").and_then(Value::as_str).unwrap_or_default(),
            r.get("kind").and_then(Value::as_str).unwrap_or_default(),
            r.get("filePath")
                .and_then(Value::as_str)
                .unwrap_or_default()
        );
        seen.insert(key)
    });
}

fn is_symbol_label(label: &str) -> bool {
    !matches!(label, "File" | "Folder" | "Community" | "Process")
}

fn is_test_file_path(file_path: &str) -> bool {
    let path = file_path.to_ascii_lowercase().replace('\\', "/");
    path.contains(".test.")
        || path.contains(".spec.")
        || path.contains("/__tests__/")
        || path.contains("/__mocks__/")
        || path.contains("/test/")
        || path.contains("/tests/")
        || path.contains("/testing/")
        || path.contains("/fixtures/")
        || path.ends_with("_test.go")
        || path.ends_with("_test.py")
        || path.contains("/test_")
        || path.contains("/conftest.")
}

fn collect_file_edits(
    repo_path: &Path,
    rel_file_path: &str,
    old_name: &str,
    new_name: &str,
    confidence: &str,
) -> Result<Option<Vec<RenameEdit>>> {
    let abs = repo_path.join(rel_file_path);
    let raw = match std::fs::read_to_string(&abs) {
        Ok(raw) => raw,
        Err(_) => return Ok(None),
    };

    let mut edits = Vec::<RenameEdit>::new();
    for (index, line) in raw.lines().enumerate() {
        let (updated, replaced) = replace_identifier_occurrences(line, old_name, new_name);
        if replaced == 0 {
            continue;
        }
        edits.push(RenameEdit {
            line: index + 1,
            old_text: line.trim().to_string(),
            new_text: updated.trim().to_string(),
            confidence: confidence.to_string(),
        });
    }

    if edits.is_empty() {
        Ok(None)
    } else {
        Ok(Some(edits))
    }
}

fn replace_identifier_occurrences(input: &str, old_name: &str, new_name: &str) -> (String, usize) {
    if old_name.is_empty() || old_name == new_name {
        return (input.to_string(), 0);
    }

    let mut out = String::with_capacity(input.len());
    let mut search_start = 0usize;
    let mut replaced = 0usize;

    while let Some(found) = input[search_start..].find(old_name) {
        let absolute = search_start + found;
        let end = absolute + old_name.len();

        let prev = input[..absolute].chars().next_back();
        let next = input[end..].chars().next();
        let prev_is_ident = prev.is_some_and(is_ident_char);
        let next_is_ident = next.is_some_and(is_ident_char);

        if !prev_is_ident && !next_is_ident {
            out.push_str(&input[search_start..absolute]);
            out.push_str(new_name);
            replaced += 1;
        } else {
            out.push_str(&input[search_start..end]);
        }
        search_start = end;
    }

    if search_start < input.len() {
        out.push_str(&input[search_start..]);
    }

    (out, replaced)
}

fn is_ident_char(ch: char) -> bool {
    ch == '_' || ch == '$' || ch.is_ascii_alphanumeric()
}

fn path_matches_changed_file(symbol_file_path: &str, changed_file_path: &str) -> bool {
    let symbol = symbol_file_path.replace('\\', "/");
    let changed = changed_file_path.replace('\\', "/");

    symbol == changed
        || symbol.ends_with(&format!("/{changed}"))
        || symbol.contains(&changed)
        || changed.contains(&symbol)
}

fn is_write_cypher(query: &str) -> bool {
    let upper = query.to_ascii_uppercase();
    CYPHER_WRITE_KEYWORDS
        .iter()
        .any(|keyword| upper.contains(keyword))
}

fn evaluate_cypher(graph: &LocalGraph, raw_query: &str) -> Result<CypherTable> {
    let query = raw_query.trim();
    if query.is_empty() {
        bail!("cypher query cannot be empty");
    }

    let (query_no_limit, limit) = split_limit(query)?;

    if query_no_limit.contains("-[") && query_no_limit.contains("]->") {
        evaluate_edge_query(graph, &query_no_limit, limit)
    } else {
        evaluate_node_query(graph, &query_no_limit, limit)
    }
}

fn split_limit(query: &str) -> Result<(String, usize)> {
    if let Some(idx) = find_keyword_ci(query, " LIMIT ") {
        let body = query[..idx].trim();
        let raw_limit = query[idx + " LIMIT ".len()..].trim();
        if raw_limit.is_empty() || !raw_limit.chars().all(|ch| ch.is_ascii_digit()) {
            bail!("invalid LIMIT clause: {raw_limit}");
        }

        let limit = raw_limit.parse::<usize>().unwrap_or(100).max(1);
        return Ok((body.to_string(), limit));
    }

    Ok((query.to_string(), 100))
}

fn evaluate_node_query(graph: &LocalGraph, query: &str, limit: usize) -> Result<CypherTable> {
    let (match_part, where_clause, return_clause) = split_match_where_return(query)?;
    let (var, label) = parse_node_pattern(&match_part)?;

    let conditions = parse_conditions(where_clause.as_deref())?;
    let return_spec = parse_return_clause(return_clause.trim())?;

    let mut count = 0usize;
    let mut rows = Vec::<Vec<Value>>::new();

    for node in &graph.data.graph.nodes {
        if let Some(required_label) = &label
            && node.label.to_ascii_lowercase() != *required_label
        {
            continue;
        }

        let mut ctx = HashMap::<String, Entity<'_>>::new();
        ctx.insert(var.clone(), Entity::Node(node));
        if !evaluate_conditions(&ctx, &conditions) {
            continue;
        }

        match &return_spec {
            CypherReturnSpec::Count { .. } => {
                count += 1;
            }
            CypherReturnSpec::Columns(columns) => {
                rows.push(build_row(&ctx, columns));
            }
        }

        if rows.len() >= limit {
            break;
        }
    }

    match return_spec {
        CypherReturnSpec::Count { alias } => Ok(CypherTable {
            headers: vec![alias],
            rows: vec![vec![Value::Number((count as u64).into())]],
        }),
        CypherReturnSpec::Columns(columns) => Ok(CypherTable {
            headers: columns.iter().map(|c| c.alias.clone()).collect::<Vec<_>>(),
            rows,
        }),
    }
}

fn evaluate_edge_query(graph: &LocalGraph, query: &str, limit: usize) -> Result<CypherTable> {
    let (match_part, where_clause, return_clause) = split_match_where_return(query)?;
    let edge = parse_edge_pattern(&match_part)?;

    let conditions = parse_conditions(where_clause.as_deref())?;
    let return_spec = parse_return_clause(return_clause.trim())?;

    let mut count = 0usize;
    let mut rows = Vec::<Vec<Value>>::new();

    for rel in &graph.data.graph.relationships {
        if let Some(required_rel_label) = &edge.rel_label
            && required_rel_label != "coderelation"
        {
            continue;
        }
        if let Some(required_type) = &edge.rel_type_filter
            && rel.rel_type.to_ascii_uppercase() != required_type.to_ascii_uppercase()
        {
            continue;
        }

        let Some(left) = graph.nodes_by_id.get(&rel.source_id) else {
            continue;
        };
        let Some(right) = graph.nodes_by_id.get(&rel.target_id) else {
            continue;
        };

        if let Some(required_left) = &edge.left_label
            && left.label.to_ascii_lowercase() != *required_left
        {
            continue;
        }
        if let Some(required_right) = &edge.right_label
            && right.label.to_ascii_lowercase() != *required_right
        {
            continue;
        }

        let mut ctx = HashMap::<String, Entity<'_>>::new();
        ctx.insert(edge.left_var.clone(), Entity::Node(left));
        ctx.insert(edge.right_var.clone(), Entity::Node(right));
        ctx.insert(edge.rel_var.clone(), Entity::Rel(rel));

        if !evaluate_conditions(&ctx, &conditions) {
            continue;
        }

        match &return_spec {
            CypherReturnSpec::Count { .. } => {
                count += 1;
            }
            CypherReturnSpec::Columns(columns) => {
                rows.push(build_row(&ctx, columns));
            }
        }

        if rows.len() >= limit {
            break;
        }
    }

    match return_spec {
        CypherReturnSpec::Count { alias } => Ok(CypherTable {
            headers: vec![alias],
            rows: vec![vec![Value::Number((count as u64).into())]],
        }),
        CypherReturnSpec::Columns(columns) => Ok(CypherTable {
            headers: columns.iter().map(|c| c.alias.clone()).collect::<Vec<_>>(),
            rows,
        }),
    }
}

fn parse_conditions(where_clause: Option<&str>) -> Result<Vec<CypherCond>> {
    let Some(where_clause) = where_clause else {
        return Ok(Vec::new());
    };

    if where_clause.trim().is_empty() {
        return Ok(Vec::new());
    }

    let mut conditions = Vec::<CypherCond>::new();
    for raw in split_ci(where_clause, " AND ") {
        let chunk = raw.trim();
        if chunk.is_empty() {
            continue;
        }

        let (left, op, right) = if let Some(idx) = find_keyword_ci(chunk, " CONTAINS ") {
            let left = chunk[..idx].trim();
            let right = chunk[idx + " CONTAINS ".len()..].trim();
            (left, CypherOp::Contains, right)
        } else if let Some(idx) = chunk.find('=') {
            let left = chunk[..idx].trim();
            let right = chunk[idx + 1..].trim();
            (left, CypherOp::Eq, right)
        } else {
            bail!("unsupported WHERE condition: {chunk}");
        };

        let (var, field) = parse_var_field(left)
            .with_context(|| format!("unsupported WHERE condition: {chunk}"))?;
        let value = parse_quoted_literal(right)
            .with_context(|| format!("unsupported WHERE condition: {chunk}"))?;

        conditions.push(CypherCond {
            var,
            field,
            op,
            value,
        });
    }

    Ok(conditions)
}

fn parse_return_clause(raw: &str) -> Result<CypherReturnSpec> {
    let trimmed = raw.trim();
    let upper = trimmed.to_ascii_uppercase();

    if upper == "COUNT(*)" {
        return Ok(CypherReturnSpec::Count {
            alias: "count".to_string(),
        });
    }

    if let Some(idx) = find_keyword_ci(trimmed, " AS ") {
        let lhs = trimmed[..idx].trim();
        let rhs = trimmed[idx + " AS ".len()..].trim();
        if lhs.eq_ignore_ascii_case("COUNT(*)") && is_valid_ident(rhs) {
            return Ok(CypherReturnSpec::Count {
                alias: rhs.to_string(),
            });
        }
    }

    let mut columns = Vec::<CypherReturnExpr>::new();
    for raw_expr in trimmed.split(',') {
        let expr = raw_expr.trim();
        if expr.is_empty() {
            continue;
        }

        let (lhs, alias_opt) = if let Some(idx) = find_keyword_ci(expr, " AS ") {
            let lhs = expr[..idx].trim().to_string();
            let alias = expr[idx + " AS ".len()..].trim().to_string();
            if alias.is_empty() || !is_valid_ident(&alias) {
                bail!("unsupported RETURN expression: {expr}");
            }
            (lhs, Some(alias))
        } else {
            (expr.to_string(), None)
        };

        let Some((var, field)) = parse_var_field(&lhs) else {
            bail!("unsupported RETURN expression: {expr}");
        };

        let alias = alias_opt.unwrap_or_else(|| format!("{var}.{field}"));

        columns.push(CypherReturnExpr { var, field, alias });
    }

    if columns.is_empty() {
        bail!("RETURN clause cannot be empty");
    }

    Ok(CypherReturnSpec::Columns(columns))
}

fn split_match_where_return(query: &str) -> Result<(String, Option<String>, String)> {
    let trimmed = query.trim();
    if !trimmed.to_ascii_uppercase().starts_with("MATCH ") {
        bail!("cypher must start with MATCH");
    }

    let Some(return_idx) = find_keyword_ci(trimmed, " RETURN ") else {
        bail!("cypher query must contain RETURN");
    };

    let after_match = trimmed["MATCH ".len()..return_idx].trim();
    let return_clause = trimmed[return_idx + " RETURN ".len()..].trim();
    if return_clause.is_empty() {
        bail!("RETURN clause cannot be empty");
    }

    if let Some(where_idx) = find_keyword_ci(after_match, " WHERE ") {
        let match_part = after_match[..where_idx].trim().to_string();
        let where_clause = after_match[where_idx + " WHERE ".len()..]
            .trim()
            .to_string();
        if match_part.is_empty() {
            bail!("MATCH pattern cannot be empty");
        }
        let where_clause = if where_clause.is_empty() {
            None
        } else {
            Some(where_clause)
        };
        Ok((match_part, where_clause, return_clause.to_string()))
    } else {
        Ok((after_match.to_string(), None, return_clause.to_string()))
    }
}

fn parse_node_pattern(pattern: &str) -> Result<(String, Option<String>)> {
    let trimmed = pattern.trim();
    if !(trimmed.starts_with('(') && trimmed.ends_with(')')) {
        bail!("node MATCH pattern must be in parentheses");
    }
    let inner = &trimmed[1..trimmed.len() - 1];
    let Some((var, label)) = parse_var_label(inner) else {
        bail!("invalid node pattern: {pattern}");
    };
    Ok((var, label))
}

fn parse_edge_pattern(pattern: &str) -> Result<EdgePattern> {
    let compact = pattern
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect::<String>();
    let Some(mid_idx) = compact.find(")-[") else {
        bail!("invalid relationship pattern: {pattern}");
    };
    let Some(rel_tail_idx) = compact[mid_idx + 3..].find("]->") else {
        bail!("invalid relationship pattern: {pattern}");
    };
    let rel_tail_idx = mid_idx + 3 + rel_tail_idx;

    let left_atom = &compact[..mid_idx + 1];
    let rel_atom = &compact[mid_idx + 3..rel_tail_idx];
    let right_atom = &compact[rel_tail_idx + 3..];

    let (left_var, left_label) = parse_node_pattern(left_atom)?;
    let (right_var, right_label) = parse_node_pattern(right_atom)?;
    let (rel_var, rel_label, rel_type_filter) = parse_rel_pattern(rel_atom)?;

    Ok(EdgePattern {
        left_var,
        left_label,
        rel_var,
        rel_label,
        rel_type_filter,
        right_var,
        right_label,
    })
}

fn parse_rel_pattern(atom: &str) -> Result<(String, Option<String>, Option<String>)> {
    let (head, props) = if let Some(brace_idx) = atom.find('{') {
        let head = atom[..brace_idx].trim();
        let props = atom[brace_idx..].trim();
        (head, Some(props))
    } else {
        (atom.trim(), None)
    };

    let Some((var, label)) = parse_var_label(head) else {
        bail!("invalid relationship atom: {atom}");
    };

    let mut rel_type_filter = None;
    if let Some(props) = props {
        if !(props.starts_with('{') && props.ends_with('}')) {
            bail!("invalid relationship property filter: {atom}");
        }
        let inside = &props[1..props.len() - 1];
        for piece in inside.split(',') {
            let piece = piece.trim();
            if piece.is_empty() {
                continue;
            }
            let Some((key, raw_value)) = piece.split_once(':') else {
                continue;
            };
            if key.trim().eq_ignore_ascii_case("type") {
                rel_type_filter = Some(parse_quoted_literal(raw_value.trim())?);
            }
        }
    }

    Ok((var, label, rel_type_filter))
}

fn parse_var_label(raw: &str) -> Option<(String, Option<String>)> {
    let token = raw.trim().trim_matches('`');
    if token.is_empty() {
        return None;
    }

    if let Some((var, label)) = token.split_once(':') {
        let var = var.trim();
        let label = label.trim().trim_matches('`');
        if !is_valid_ident(var) {
            return None;
        }
        if label.is_empty() {
            Some((var.to_string(), None))
        } else {
            Some((var.to_string(), Some(label.to_ascii_lowercase())))
        }
    } else {
        if !is_valid_ident(token) {
            return None;
        }
        Some((token.to_string(), None))
    }
}

fn parse_var_field(raw: &str) -> Option<(String, String)> {
    let (var, field) = raw.split_once('.')?;
    let var = var.trim();
    let field = field.trim();
    if !is_valid_ident(var) || !is_valid_ident(field) {
        return None;
    }
    Some((var.to_string(), field.to_string()))
}

fn parse_quoted_literal(raw: &str) -> Result<String> {
    let value = raw.trim();
    if value.len() < 2 {
        bail!("invalid quoted literal: {raw}");
    }

    let first = value.chars().next().unwrap_or_default();
    let last = value.chars().last().unwrap_or_default();
    if (first == '\'' || first == '"') && first == last {
        Ok(value[1..value.len() - 1].to_string())
    } else {
        bail!("invalid quoted literal: {raw}");
    }
}

fn find_keyword_ci(haystack: &str, needle: &str) -> Option<usize> {
    haystack
        .to_ascii_uppercase()
        .find(&needle.to_ascii_uppercase())
}

fn split_ci<'a>(input: &'a str, sep: &str) -> Vec<&'a str> {
    let mut parts = Vec::<&'a str>::new();
    let mut cursor = input;
    loop {
        if let Some(idx) = find_keyword_ci(cursor, sep) {
            let (head, tail) = cursor.split_at(idx);
            parts.push(head);
            cursor = &tail[sep.len()..];
        } else {
            parts.push(cursor);
            break;
        }
    }
    parts
}

fn is_valid_ident(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first == '_' || first.is_ascii_alphabetic()) {
        return false;
    }
    chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn evaluate_conditions(ctx: &HashMap<String, Entity<'_>>, conditions: &[CypherCond]) -> bool {
    for cond in conditions {
        let Some(entity) = ctx.get(&cond.var) else {
            return false;
        };

        let Some(actual) = entity_field(entity, &cond.field) else {
            return false;
        };

        let actual = value_to_string(&actual);
        let expected = cond.value.as_str();
        let matched = match cond.op {
            CypherOp::Eq => actual == expected,
            CypherOp::Contains => actual
                .to_ascii_lowercase()
                .contains(&expected.to_ascii_lowercase()),
        };

        if !matched {
            return false;
        }
    }

    true
}

fn build_row(ctx: &HashMap<String, Entity<'_>>, columns: &[CypherReturnExpr]) -> Vec<Value> {
    columns
        .iter()
        .map(|expr| {
            ctx.get(&expr.var)
                .and_then(|entity| entity_field(entity, &expr.field))
                .unwrap_or(Value::Null)
        })
        .collect::<Vec<_>>()
}

fn entity_field(entity: &Entity<'_>, raw_field: &str) -> Option<Value> {
    let field = raw_field.to_ascii_lowercase();
    match entity {
        Entity::Node(node) => match field.as_str() {
            "id" => Some(Value::String(node.id.clone())),
            "name" => Some(Value::String(node.name.clone())),
            "label" | "type" => Some(Value::String(node.label.clone())),
            "filepath" | "file_path" => Some(Value::String(node.file_path.clone())),
            "startline" | "start_line" => node.start_line.map(|v| Value::Number(v.into())),
            "language" => node.language.clone().map(Value::String),
            _ => None,
        },
        Entity::Rel(rel) => match field.as_str() {
            "id" => Some(Value::String(rel.id.clone())),
            "sourceid" | "source_id" => Some(Value::String(rel.source_id.clone())),
            "targetid" | "target_id" => Some(Value::String(rel.target_id.clone())),
            "type" | "rel_type" => Some(Value::String(rel.rel_type.clone())),
            "confidence" => serde_json::Number::from_f64(rel.confidence as f64).map(Value::Number),
            "reason" => Some(Value::String(rel.reason.clone())),
            _ => None,
        },
    }
}

fn value_to_string(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::Bool(v) => v.to_string(),
        Value::Number(v) => v.to_string(),
        Value::String(v) => v.clone(),
        Value::Array(v) => serde_json::to_string(v).unwrap_or_default(),
        Value::Object(v) => serde_json::to_string(v).unwrap_or_default(),
    }
}

fn render_markdown_table(table: &CypherTable) -> String {
    if table.rows.is_empty() {
        return "".to_string();
    }

    let mut lines = Vec::<String>::new();
    lines.push(format!("| {} |", table.headers.join(" | ")));
    lines.push(format!(
        "| {} |",
        table
            .headers
            .iter()
            .map(|_| "---")
            .collect::<Vec<_>>()
            .join(" | ")
    ));

    for row in &table.rows {
        let mut cells = Vec::<String>::new();
        for cell in row {
            let text = value_to_string(cell)
                .replace('|', r"\|")
                .replace('\n', "\\n");
            cells.push(text);
        }
        lines.push(format!("| {} |", cells.join(" | ")));
    }

    lines.join("\n")
}
