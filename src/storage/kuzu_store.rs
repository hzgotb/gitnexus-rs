use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use kuzu::{Connection, Database, SystemConfig, Value as KuzuValue};
use serde_json::{Map, Number, Value, json};

use crate::ingestion::{HeuristicCommunity, HeuristicProcess, IngestionResult, StructureNode};

use super::repo_manager::{StoragePaths, get_storage_paths};

const CODE_RELATION_TABLE: &str = "CodeRelation";
const SYMBOL_NODE_LABELS: [&str; 23] = [
    "Function",
    "Class",
    "Interface",
    "Method",
    "CodeElement",
    "Struct",
    "Enum",
    "Macro",
    "Typedef",
    "Union",
    "Namespace",
    "Trait",
    "Impl",
    "TypeAlias",
    "Const",
    "Static",
    "Property",
    "Record",
    "Delegate",
    "Annotation",
    "Constructor",
    "Template",
    "Module",
];
const NODE_TABLES: [&str; 27] = [
    "File",
    "Folder",
    "Function",
    "Class",
    "Interface",
    "Method",
    "CodeElement",
    "Community",
    "Process",
    "Struct",
    "Enum",
    "Macro",
    "Typedef",
    "Union",
    "Namespace",
    "Trait",
    "Impl",
    "TypeAlias",
    "Const",
    "Static",
    "Property",
    "Record",
    "Delegate",
    "Annotation",
    "Constructor",
    "Template",
    "Module",
];
const FTS_TARGETS: [(&str, &str, &[&str]); 24] = [
    ("File", "file_fts", &["name", "content"]),
    ("Function", "function_fts", &["name", "content"]),
    ("Class", "class_fts", &["name", "content"]),
    ("Interface", "interface_fts", &["name", "content"]),
    ("Method", "method_fts", &["name", "content"]),
    ("CodeElement", "code_element_fts", &["name", "content"]),
    ("Struct", "struct_fts", &["name", "content"]),
    ("Enum", "enum_fts", &["name", "content"]),
    ("Macro", "macro_fts", &["name", "content"]),
    ("Typedef", "typedef_fts", &["name", "content"]),
    ("Union", "union_fts", &["name", "content"]),
    ("Namespace", "namespace_fts", &["name", "content"]),
    ("Trait", "trait_fts", &["name", "content"]),
    ("Impl", "impl_fts", &["name", "content"]),
    ("TypeAlias", "type_alias_fts", &["name", "content"]),
    ("Const", "const_fts", &["name", "content"]),
    ("Static", "static_fts", &["name", "content"]),
    ("Property", "property_fts", &["name", "content"]),
    ("Record", "record_fts", &["name", "content"]),
    ("Delegate", "delegate_fts", &["name", "content"]),
    ("Annotation", "annotation_fts", &["name", "content"]),
    ("Constructor", "constructor_fts", &["name", "content"]),
    ("Template", "template_fts", &["name", "content"]),
    ("Module", "module_fts", &["name", "content"]),
];

#[derive(Debug, Clone, Default)]
pub struct KuzuBuildReport {
    pub indexed_nodes: u64,
    pub indexed_edges: u64,
    pub fts_indexes: u64,
    pub fts_warnings: Vec<String>,
}

#[derive(Debug)]
struct GraphIndex {
    node_ids: HashSet<String>,
    node_names: HashMap<String, String>,
    symbol_ids_by_file: HashMap<String, Vec<String>>,
    indexed_nodes: u64,
    indexed_edges: u64,
}

#[derive(Debug, Clone)]
struct SearchMatch {
    id: String,
    name: String,
    node_type: String,
    file_path: String,
    start_line: Option<u64>,
    end_line: Option<u64>,
    content: Option<String>,
    score: f64,
}

pub fn rebuild_from_ingestion(
    repo_path: &Path,
    storage: &StoragePaths,
    ingestion: &IngestionResult,
) -> Result<KuzuBuildReport> {
    cleanup_kuzu_artifacts(storage)?;

    std::fs::create_dir_all(&storage.storage_path).with_context(|| {
        format!(
            "failed to create storage directory {}",
            storage.storage_path.to_string_lossy()
        )
    })?;

    let db = Database::new(&storage.kuzu_path, SystemConfig::default()).with_context(|| {
        format!(
            "failed to create kuzu database at {}",
            storage.kuzu_path.to_string_lossy()
        )
    })?;
    let conn = Connection::new(&db).context("failed to open kuzu connection")?;

    create_schema(&conn)?;
    let graph_index = load_ingestion_into_kuzu(&conn, repo_path, ingestion)?;
    let (fts_indexes, fts_warnings) = build_fts_indexes(&conn);

    Ok(KuzuBuildReport {
        indexed_nodes: graph_index.indexed_nodes,
        indexed_edges: graph_index.indexed_edges,
        fts_indexes,
        fts_warnings,
    })
}

pub fn try_search_fts(repo_path: &Path, query: &str, limit: usize) -> Result<Option<Value>> {
    let storage = get_storage_paths(repo_path);
    if !storage.kuzu_path.exists() {
        return Ok(None);
    }

    let db = match Database::new(&storage.kuzu_path, SystemConfig::default().read_only(true)) {
        Ok(db) => db,
        Err(_) => return Ok(None),
    };
    let conn = match Connection::new(&db) {
        Ok(conn) => conn,
        Err(_) => return Ok(None),
    };

    if load_fts_extension(&conn, false).is_err() {
        return Ok(None);
    }

    let mut matches = Vec::<SearchMatch>::new();
    let mut successful_queries = 0usize;
    for (table, index_name, _properties) in FTS_TARGETS {
        let query_rows = match query_fts_index(&conn, table, index_name, query, limit) {
            Ok(rows) => rows,
            Err(err) => {
                let msg = err.to_string();
                if msg.contains("does not exist")
                    || msg.contains("not found")
                    || msg.contains("No function matches")
                {
                    continue;
                }
                return Ok(None);
            }
        };

        successful_queries += 1;
        for row in query_rows {
            let Some(node) = row.get("node").and_then(Value::as_object) else {
                continue;
            };

            let id = node.get("id").and_then(value_as_string).unwrap_or_default();
            if id.is_empty() {
                continue;
            }

            matches.push(SearchMatch {
                id,
                name: node
                    .get("name")
                    .and_then(value_as_string)
                    .unwrap_or_default(),
                node_type: table.to_string(),
                file_path: node
                    .get("filePath")
                    .and_then(value_as_string)
                    .unwrap_or_default(),
                start_line: node.get("startLine").and_then(value_as_u64),
                end_line: node.get("endLine").and_then(value_as_u64),
                content: node.get("content").and_then(value_as_string),
                score: row.get("score").and_then(value_as_f64).unwrap_or(0.0),
            });
        }
    }

    if successful_queries == 0 {
        return Ok(None);
    }

    let deduped = dedupe_search_matches(matches);
    let definitions = deduped
        .into_iter()
        .take(limit.max(1))
        .map(search_match_to_value)
        .collect::<Vec<_>>();

    Ok(Some(json!({
        "processes": [],
        "process_symbols": [],
        "definitions": definitions,
    })))
}

fn cleanup_kuzu_artifacts(storage: &StoragePaths) -> Result<()> {
    remove_path_if_exists(&storage.kuzu_path)?;
    remove_path_if_exists(&storage.storage_path.join("kuzu.wal"))?;
    remove_path_if_exists(&storage.storage_path.join("kuzu.lock"))?;
    Ok(())
}

fn remove_path_if_exists(path: &Path) -> Result<()> {
    if !path.exists() {
        return Ok(());
    }

    let metadata = std::fs::metadata(path)
        .with_context(|| format!("failed to read metadata for {}", path.to_string_lossy()))?;
    if metadata.is_dir() {
        std::fs::remove_dir_all(path)
            .with_context(|| format!("failed to remove {}", path.to_string_lossy()))?;
    } else {
        std::fs::remove_file(path)
            .with_context(|| format!("failed to remove {}", path.to_string_lossy()))?;
    }
    Ok(())
}

fn create_schema(conn: &Connection<'_>) -> Result<()> {
    for query in node_schema_queries() {
        conn.query(&query)
            .with_context(|| format!("failed to create kuzu schema: {}", truncate(&query, 160)))?;
    }

    let relation_query = build_relation_schema_query();
    conn.query(&relation_query).with_context(|| {
        format!(
            "failed to create kuzu relation schema: {}",
            truncate(&relation_query, 160)
        )
    })?;

    Ok(())
}

fn node_schema_queries() -> Vec<String> {
    let mut queries = Vec::<String>::new();
    queries.push(
        "CREATE NODE TABLE File (id STRING, name STRING, filePath STRING, content STRING, PRIMARY KEY (id))"
            .to_string(),
    );
    queries.push(
        "CREATE NODE TABLE Folder (id STRING, name STRING, filePath STRING, PRIMARY KEY (id))"
            .to_string(),
    );
    queries.push(
        "CREATE NODE TABLE Community (id STRING, label STRING, heuristicLabel STRING, symbolCount INT32, PRIMARY KEY (id))"
            .to_string(),
    );
    queries.push(
        "CREATE NODE TABLE Process (id STRING, label STRING, heuristicLabel STRING, processType STRING, stepCount INT32, entryPointId STRING, terminalId STRING, PRIMARY KEY (id))"
            .to_string(),
    );

    for label in SYMBOL_NODE_LABELS {
        queries.push(format!(
            "CREATE NODE TABLE {} (id STRING, name STRING, filePath STRING, startLine INT64, endLine INT64, content STRING, PRIMARY KEY (id))",
            label_expr(label)
        ));
    }

    queries
}

fn build_relation_schema_query() -> String {
    let pairs = NODE_TABLES
        .iter()
        .flat_map(|source| {
            NODE_TABLES.iter().map(move |target| {
                format!("  FROM {} TO {}", label_expr(source), label_expr(target))
            })
        })
        .collect::<Vec<_>>()
        .join(",\n");

    format!(
        "CREATE REL TABLE {CODE_RELATION_TABLE} (\n{pairs},\n  type STRING,\n  confidence DOUBLE,\n  reason STRING,\n  step INT32\n)"
    )
}

fn load_ingestion_into_kuzu(
    conn: &Connection<'_>,
    repo_path: &Path,
    ingestion: &IngestionResult,
) -> Result<GraphIndex> {
    let mut graph_index = GraphIndex {
        node_ids: HashSet::new(),
        node_names: HashMap::new(),
        symbol_ids_by_file: HashMap::new(),
        indexed_nodes: 0,
        indexed_edges: 0,
    };

    for node in &ingestion.graph.nodes {
        let properties = graph_node_properties(repo_path, node);
        create_node(conn, &node.label, &properties)?;
        graph_index.node_ids.insert(node.id.clone());
        graph_index
            .node_names
            .insert(node.id.clone(), node.name.clone());
        if is_symbol_label(&node.label) {
            graph_index
                .symbol_ids_by_file
                .entry(node.file_path.clone())
                .or_default()
                .push(node.id.clone());
        }
        graph_index.indexed_nodes += 1;
    }

    for community in &ingestion.communities {
        let heuristic_label = community_heuristic_label(community);
        create_node(
            conn,
            "Community",
            &[
                ("id", Value::String(community.id.clone())),
                ("label", Value::String(community.id.clone())),
                ("heuristicLabel", Value::String(heuristic_label)),
                (
                    "symbolCount",
                    Value::Number(Number::from(community.file_count.min(i32::MAX as u64))),
                ),
            ],
        )?;
        graph_index.node_ids.insert(community.id.clone());
        graph_index
            .node_names
            .insert(community.id.clone(), community.id.clone());
        graph_index.indexed_nodes += 1;
    }

    for process in &ingestion.processes {
        let ordered_symbols = ordered_process_symbols(process);
        let heuristic_label = process_heuristic_label(process, &graph_index.node_names);
        let terminal_id = ordered_symbols
            .last()
            .cloned()
            .unwrap_or_else(|| process.entry_symbol_id.clone());
        create_node(
            conn,
            "Process",
            &[
                ("id", Value::String(process.id.clone())),
                ("label", Value::String(process.id.clone())),
                ("heuristicLabel", Value::String(heuristic_label)),
                ("processType", Value::String("heuristic".to_string())),
                (
                    "stepCount",
                    Value::Number(Number::from(process.step_count.min(i32::MAX as u64))),
                ),
                (
                    "entryPointId",
                    Value::String(process.entry_symbol_id.clone()),
                ),
                ("terminalId", Value::String(terminal_id)),
            ],
        )?;
        graph_index.node_ids.insert(process.id.clone());
        graph_index
            .node_names
            .insert(process.id.clone(), process.id.clone());
        graph_index.indexed_nodes += 1;
    }

    for rel in &ingestion.graph.relationships {
        if !graph_index.node_ids.contains(&rel.source_id)
            || !graph_index.node_ids.contains(&rel.target_id)
        {
            continue;
        }
        create_relation(
            conn,
            &rel.source_id,
            &rel.target_id,
            &rel.rel_type,
            rel.confidence as f64,
            &rel.reason,
            None,
        )?;
        graph_index.indexed_edges += 1;
    }

    for community in &ingestion.communities {
        for file in &community.files {
            if let Some(symbol_ids) = graph_index.symbol_ids_by_file.get(file) {
                for symbol_id in symbol_ids {
                    create_relation(
                        conn,
                        symbol_id,
                        &community.id,
                        "MEMBER_OF",
                        1.0,
                        "heuristic community membership",
                        None,
                    )?;
                    graph_index.indexed_edges += 1;
                }
            }
        }
    }

    for process in &ingestion.processes {
        for (idx, symbol_id) in ordered_process_symbols(process).into_iter().enumerate() {
            if !graph_index.node_ids.contains(&symbol_id) {
                continue;
            }
            create_relation(
                conn,
                &symbol_id,
                &process.id,
                "STEP_IN_PROCESS",
                1.0,
                "heuristic process step",
                Some((idx + 1) as i32),
            )?;
            graph_index.indexed_edges += 1;
        }
    }

    Ok(graph_index)
}

fn build_fts_indexes(conn: &Connection<'_>) -> (u64, Vec<String>) {
    let mut warnings = Vec::<String>::new();
    if let Err(err) = load_fts_extension(conn, true) {
        warnings.push(err.to_string());
        return (0, warnings);
    }

    let mut created = 0u64;
    for (table, index_name, properties) in FTS_TARGETS {
        let props = properties
            .iter()
            .map(|prop| format!("'{prop}'"))
            .collect::<Vec<_>>()
            .join(", ");
        let query = format!(
            "CALL CREATE_FTS_INDEX('{table}', '{index_name}', [{props}], stemmer := 'porter')"
        );
        match conn.query(&query) {
            Ok(_) => created += 1,
            Err(err) => {
                let msg = err.to_string();
                if !msg.contains("already exists") {
                    warnings.push(format!("{table}/{index_name}: {msg}"));
                }
            }
        }
    }

    (created, warnings)
}

fn load_fts_extension(conn: &Connection<'_>, allow_install: bool) -> Result<()> {
    if allow_install {
        match conn.query("INSTALL fts") {
            Ok(_) => {}
            Err(err) => {
                let msg = err.to_string();
                if !msg.contains("already loaded")
                    && !msg.contains("already installed")
                    && !msg.contains("already exists")
                {
                    return Err(err).context("failed to install kuzu fts extension");
                }
            }
        }
    }

    match conn.query("LOAD EXTENSION fts") {
        Ok(_) => Ok(()),
        Err(err) => {
            let msg = err.to_string();
            if msg.contains("already loaded") || msg.contains("already exists") {
                Ok(())
            } else {
                Err(err).context("failed to load kuzu fts extension")
            }
        }
    }
}

fn query_fts_index(
    conn: &Connection<'_>,
    table: &str,
    index_name: &str,
    query: &str,
    limit: usize,
) -> Result<Vec<Map<String, Value>>> {
    let escaped_query = escape_cypher_string(query);
    let cypher = format!(
        "CALL QUERY_FTS_INDEX('{table}', '{index_name}', '{escaped_query}', conjunctive := false)
         RETURN node, score
         ORDER BY score DESC
         LIMIT {}",
        limit.max(1)
    );
    query_objects(conn, &cypher)
}

fn query_objects(conn: &Connection<'_>, query: &str) -> Result<Vec<Map<String, Value>>> {
    let mut result = conn
        .query(query)
        .with_context(|| format!("native kuzu query failed: {}", truncate(query, 180)))?;
    let headers = result.get_column_names();

    let mut rows = Vec::<Map<String, Value>>::new();
    for row in result.by_ref() {
        let mut object = Map::<String, Value>::new();
        for (idx, value) in row.into_iter().enumerate() {
            let key = headers
                .get(idx)
                .cloned()
                .unwrap_or_else(|| format!("col_{idx}"));
            object.insert(key, kuzu_value_to_json(value));
        }
        rows.push(object);
    }

    Ok(rows)
}

fn create_node(conn: &Connection<'_>, label: &str, properties: &[(&str, Value)]) -> Result<()> {
    let props = cypher_map(properties);
    let query = format!("CREATE (n:{} {{{props}}})", label_expr(label));
    conn.query(&query)
        .with_context(|| format!("failed to create kuzu node: {}", truncate(&query, 180)))?;
    Ok(())
}

fn create_relation(
    conn: &Connection<'_>,
    source_id: &str,
    target_id: &str,
    rel_type: &str,
    confidence: f64,
    reason: &str,
    step: Option<i32>,
) -> Result<()> {
    let mut props = vec![
        ("type", Value::String(rel_type.to_string())),
        (
            "confidence",
            Number::from_f64(confidence)
                .map(Value::Number)
                .unwrap_or_else(|| Value::Number(Number::from(1))),
        ),
        ("reason", Value::String(reason.to_string())),
    ];
    if let Some(step) = step {
        props.push(("step", Value::Number(Number::from(step))));
    }

    let query = format!(
        "MATCH (a {{id: '{source}'}}), (b {{id: '{target}'}}) CREATE (a)-[:{CODE_RELATION_TABLE} {{{props}}}]->(b)",
        source = escape_cypher_string(source_id),
        target = escape_cypher_string(target_id),
        props = cypher_map(&props),
    );

    conn.query(&query)
        .with_context(|| format!("failed to create kuzu relation: {}", truncate(&query, 180)))?;
    Ok(())
}

fn graph_node_properties(repo_path: &Path, node: &StructureNode) -> Vec<(&'static str, Value)> {
    match node.label.as_str() {
        "File" => vec![
            ("id", Value::String(node.id.clone())),
            ("name", Value::String(node.name.clone())),
            ("filePath", Value::String(node.file_path.clone())),
            (
                "content",
                Value::String(read_file_content(repo_path, &node.file_path).unwrap_or_default()),
            ),
        ],
        "Folder" => vec![
            ("id", Value::String(node.id.clone())),
            ("name", Value::String(node.name.clone())),
            ("filePath", Value::String(node.file_path.clone())),
        ],
        _ => {
            let mut props = vec![
                ("id", Value::String(node.id.clone())),
                ("name", Value::String(node.name.clone())),
                ("filePath", Value::String(node.file_path.clone())),
            ];
            if let Some(start_line) = node.start_line {
                props.push(("startLine", Value::Number(Number::from(start_line))));
                props.push(("endLine", Value::Number(Number::from(start_line))));
            }
            if let Some(content) =
                symbol_content_snippet(repo_path, &node.file_path, node.start_line)
            {
                props.push(("content", Value::String(content)));
            }
            props
        }
    }
}

fn read_file_content(repo_path: &Path, rel_file_path: &str) -> Option<String> {
    std::fs::read_to_string(repo_path.join(rel_file_path)).ok()
}

fn symbol_content_snippet(
    repo_path: &Path,
    rel_file_path: &str,
    start_line: Option<u64>,
) -> Option<String> {
    let raw = read_file_content(repo_path, rel_file_path)?;
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

fn community_heuristic_label(community: &HeuristicCommunity) -> String {
    let mut counts = BTreeMap::<String, usize>::new();
    for file in &community.files {
        let key = file
            .split('/')
            .next()
            .unwrap_or_default()
            .trim()
            .to_string();
        if !key.is_empty() {
            *counts.entry(key).or_default() += 1;
        }
    }

    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)))
        .map(|(name, _)| name)
        .filter(|name| !name.is_empty())
        .or_else(|| community.files.first().cloned())
        .unwrap_or_else(|| community.id.clone())
}

fn process_heuristic_label(
    process: &HeuristicProcess,
    node_names: &HashMap<String, String>,
) -> String {
    let entry_name = node_names
        .get(&process.entry_symbol_id)
        .cloned()
        .unwrap_or_else(|| process.entry_symbol_id.clone());
    format!("{} -> {}", process.id, entry_name)
}

fn ordered_process_symbols(process: &HeuristicProcess) -> Vec<String> {
    let mut ordered = Vec::<String>::new();
    let mut seen = HashSet::<String>::new();

    if seen.insert(process.entry_symbol_id.clone()) {
        ordered.push(process.entry_symbol_id.clone());
    }

    let mut rest = process.symbols.clone();
    rest.sort();
    for symbol_id in rest {
        if seen.insert(symbol_id.clone()) {
            ordered.push(symbol_id);
        }
    }

    ordered
}

fn dedupe_search_matches(matches: Vec<SearchMatch>) -> Vec<SearchMatch> {
    let mut best_by_id = HashMap::<String, SearchMatch>::new();

    for item in matches {
        match best_by_id.get(&item.id) {
            Some(existing) if existing.score >= item.score => {}
            _ => {
                best_by_id.insert(item.id.clone(), item);
            }
        }
    }

    let mut deduped = best_by_id.into_values().collect::<Vec<_>>();
    deduped.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.name.cmp(&b.name))
            .then_with(|| a.file_path.cmp(&b.file_path))
    });
    deduped
}

fn search_match_to_value(item: SearchMatch) -> Value {
    let mut map = Map::<String, Value>::new();
    map.insert("id".to_string(), Value::String(item.id));
    map.insert("name".to_string(), Value::String(item.name));
    map.insert("type".to_string(), Value::String(item.node_type));
    map.insert("filePath".to_string(), Value::String(item.file_path));
    if let Some(start_line) = item.start_line {
        map.insert(
            "startLine".to_string(),
            Value::Number(Number::from(start_line)),
        );
    }
    if let Some(end_line) = item.end_line {
        map.insert("endLine".to_string(), Value::Number(Number::from(end_line)));
    }
    if let Some(content) = item.content
        && !content.is_empty()
    {
        map.insert("content".to_string(), Value::String(content));
    }
    map.insert(
        "score".to_string(),
        Number::from_f64(item.score)
            .map(Value::Number)
            .unwrap_or_else(|| Value::Number(Number::from(0))),
    );
    Value::Object(map)
}

fn is_symbol_label(label: &str) -> bool {
    SYMBOL_NODE_LABELS.contains(&label)
}

fn label_expr(label: &str) -> String {
    match label {
        "Struct" | "Enum" | "Macro" | "Typedef" | "Union" | "Namespace" | "Trait" | "Impl"
        | "TypeAlias" | "Const" | "Static" | "Property" | "Record" | "Delegate" | "Annotation"
        | "Constructor" | "Template" | "Module" => format!("`{label}`"),
        _ => label.to_string(),
    }
}

fn cypher_map(properties: &[(&str, Value)]) -> String {
    properties
        .iter()
        .map(|(key, value)| format!("{key}: {}", cypher_value(value)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn cypher_value(value: &Value) -> String {
    match value {
        Value::Null => "NULL".to_string(),
        Value::Bool(v) => v.to_string(),
        Value::Number(v) => v.to_string(),
        Value::String(v) => format!("'{}'", escape_cypher_string(v)),
        Value::Array(items) => format!(
            "[{}]",
            items
                .iter()
                .map(cypher_value)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        Value::Object(_) => "'{}'".to_string(),
    }
}

fn escape_cypher_string(raw: &str) -> String {
    raw.replace('\\', "\\\\").replace('\'', "''")
}

fn truncate(text: &str, max_len: usize) -> String {
    if text.len() <= max_len {
        return text.to_string();
    }
    format!("{}...", &text[..max_len])
}

fn kuzu_value_to_json(value: KuzuValue) -> Value {
    match value {
        KuzuValue::Null(_) => Value::Null,
        KuzuValue::Bool(v) => Value::Bool(v),
        KuzuValue::Int8(v) => json!(v),
        KuzuValue::Int16(v) => json!(v),
        KuzuValue::Int32(v) => json!(v),
        KuzuValue::Int64(v) => json!(v),
        KuzuValue::UInt8(v) => json!(v),
        KuzuValue::UInt16(v) => json!(v),
        KuzuValue::UInt32(v) => json!(v),
        KuzuValue::UInt64(v) => json!(v),
        KuzuValue::Int128(v) => Value::String(v.to_string()),
        KuzuValue::Float(v) => Number::from_f64(v as f64)
            .map(Value::Number)
            .unwrap_or_else(|| Value::String(v.to_string())),
        KuzuValue::Double(v) => Number::from_f64(v)
            .map(Value::Number)
            .unwrap_or_else(|| Value::String(v.to_string())),
        KuzuValue::String(v) => Value::String(v),
        KuzuValue::Blob(v) => json!(v),
        KuzuValue::List(_, items) | KuzuValue::Array(_, items) => {
            Value::Array(items.into_iter().map(kuzu_value_to_json).collect())
        }
        KuzuValue::Struct(fields) => {
            let mut map = Map::new();
            for (name, value) in fields {
                map.insert(name, kuzu_value_to_json(value));
            }
            Value::Object(map)
        }
        KuzuValue::Map(_, pairs) => Value::Array(
            pairs
                .into_iter()
                .map(|(key, value)| {
                    json!({
                        "key": kuzu_value_to_json(key),
                        "value": kuzu_value_to_json(value),
                    })
                })
                .collect(),
        ),
        KuzuValue::Union { value, .. } => kuzu_value_to_json(*value),
        other => Value::String(other.to_string()),
    }
}

fn value_as_string(value: &Value) -> Option<String> {
    match value {
        Value::String(v) => Some(v.clone()),
        Value::Number(v) => Some(v.to_string()),
        Value::Bool(v) => Some(v.to_string()),
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}

fn value_as_u64(value: &Value) -> Option<u64> {
    match value {
        Value::Number(v) => v
            .as_u64()
            .or_else(|| v.as_i64().and_then(|n| u64::try_from(n).ok())),
        Value::String(v) => v.parse::<u64>().ok(),
        _ => None,
    }
}

fn value_as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(v) => v.as_f64(),
        Value::String(v) => v.parse::<f64>().ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::ingestion::{
        HeuristicCommunity, HeuristicProcess, IngestionResult, IngestionStats, ScannedFile,
        StructureGraph, StructureNode, StructureRelationship, SupportedLanguage,
    };

    #[test]
    fn rebuild_from_ingestion_creates_queryable_kuzu_db() {
        let root = create_temp_dir("gitnexus_rs_kuzu_store_test");
        let src_dir = root.join("src");
        std::fs::create_dir_all(&src_dir).expect("failed to create src dir");
        std::fs::write(
            src_dir.join("lib.rs"),
            "pub fn hello() {\n    world();\n}\n\nfn world() {}\n",
        )
        .expect("failed to write rust source");

        let storage = get_storage_paths(&root);
        let ingestion = sample_ingestion();
        let report = rebuild_from_ingestion(&root, &storage, &ingestion)
            .expect("failed to materialize kuzu database");

        assert!(storage.kuzu_path.exists());
        assert!(report.indexed_nodes >= 5);
        assert!(report.indexed_edges >= 4);

        let db = Database::new(&storage.kuzu_path, SystemConfig::default().read_only(true))
            .expect("failed to open kuzu database");
        let conn = Connection::new(&db).expect("failed to open kuzu connection");
        let rows = query_objects(&conn, "MATCH (n:Function) RETURN COUNT(n) AS count")
            .expect("failed to query function count");
        let count = rows[0]
            .get("count")
            .and_then(value_as_u64)
            .expect("missing function count");
        assert_eq!(count, 2);
    }

    fn sample_ingestion() -> IngestionResult {
        IngestionResult {
            scanned_files: vec![ScannedFile {
                path: "src/lib.rs".to_string(),
                size_bytes: 44,
                language: Some(SupportedLanguage::Rust),
            }],
            graph: StructureGraph {
                nodes: vec![
                    StructureNode {
                        id: "Folder:src".to_string(),
                        label: "Folder".to_string(),
                        name: "src".to_string(),
                        file_path: "src".to_string(),
                        start_line: None,
                        language: None,
                    },
                    StructureNode {
                        id: "File:src/lib.rs".to_string(),
                        label: "File".to_string(),
                        name: "lib.rs".to_string(),
                        file_path: "src/lib.rs".to_string(),
                        start_line: None,
                        language: None,
                    },
                    StructureNode {
                        id: "Function:src/lib.rs:hello".to_string(),
                        label: "Function".to_string(),
                        name: "hello".to_string(),
                        file_path: "src/lib.rs".to_string(),
                        start_line: Some(1),
                        language: Some("rust".to_string()),
                    },
                    StructureNode {
                        id: "Function:src/lib.rs:world".to_string(),
                        label: "Function".to_string(),
                        name: "world".to_string(),
                        file_path: "src/lib.rs".to_string(),
                        start_line: Some(5),
                        language: Some("rust".to_string()),
                    },
                ],
                relationships: vec![
                    StructureRelationship {
                        id: "contains".to_string(),
                        source_id: "Folder:src".to_string(),
                        target_id: "File:src/lib.rs".to_string(),
                        rel_type: "CONTAINS".to_string(),
                        confidence: 1.0,
                        reason: String::new(),
                    },
                    StructureRelationship {
                        id: "defines_hello".to_string(),
                        source_id: "File:src/lib.rs".to_string(),
                        target_id: "Function:src/lib.rs:hello".to_string(),
                        rel_type: "DEFINES".to_string(),
                        confidence: 1.0,
                        reason: String::new(),
                    },
                    StructureRelationship {
                        id: "defines_world".to_string(),
                        source_id: "File:src/lib.rs".to_string(),
                        target_id: "Function:src/lib.rs:world".to_string(),
                        rel_type: "DEFINES".to_string(),
                        confidence: 1.0,
                        reason: String::new(),
                    },
                    StructureRelationship {
                        id: "calls".to_string(),
                        source_id: "Function:src/lib.rs:hello".to_string(),
                        target_id: "Function:src/lib.rs:world".to_string(),
                        rel_type: "CALLS".to_string(),
                        confidence: 1.0,
                        reason: String::new(),
                    },
                ],
            },
            stats: IngestionStats {
                total_files: 1,
                parseable_files: 1,
                symbols: 2,
                skipped_large_files: 0,
                nodes: 4,
                edges: 4,
                communities: 1,
                processes: 1,
                language_breakdown: BTreeMap::from([("rust".to_string(), 1)]),
            },
            communities: vec![HeuristicCommunity {
                id: "community_1".to_string(),
                file_count: 1,
                files: vec!["src/lib.rs".to_string()],
            }],
            processes: vec![HeuristicProcess {
                id: "process_1".to_string(),
                entry_symbol_id: "Function:src/lib.rs:hello".to_string(),
                symbol_count: 2,
                step_count: 2,
                symbols: vec![
                    "Function:src/lib.rs:hello".to_string(),
                    "Function:src/lib.rs:world".to_string(),
                ],
            }],
        }
    }

    fn create_temp_dir(prefix: &str) -> PathBuf {
        let unique = format!(
            "{}_{}_{}",
            prefix,
            std::process::id(),
            chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
        );
        let path = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&path).expect("failed to create temporary directory");
        path
    }
}
