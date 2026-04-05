use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use kuzu::{Connection, Database, SystemConfig, Value as KuzuValue};
use serde_json::{Map, Value, json};

use crate::storage::repo_manager::{find_repo, get_storage_paths, list_registered_repos};

const DEFAULT_MAX_SYMBOLS_PER_PROCESS: usize = 10;
const DEFAULT_DEFINITION_LIMIT: usize = 20;
const DEFAULT_RELATION_TYPES: [&str; 4] = ["CALLS", "IMPORTS", "EXTENDS", "IMPLEMENTS"];
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

pub fn try_run_cypher(query: &str, repo: Option<&str>) -> Result<Option<Value>> {
    with_kuzu_connection(repo, |conn| {
        let mut result = conn
            .query(query)
            .context("native kuzu cypher query failed")?;

        let headers = result.get_column_names();
        let rows = result
            .by_ref()
            .map(|row| {
                row.into_iter()
                    .map(kuzu_value_to_json)
                    .collect::<Vec<Value>>()
            })
            .collect::<Vec<_>>();

        Ok(json!({
            "markdown": render_markdown_table(&headers, &rows),
            "row_count": rows.len(),
        }))
    })
}

pub fn try_run_query(
    search_query: &str,
    repo: Option<&str>,
    context: Option<&str>,
    goal: Option<&str>,
    limit: usize,
    include_content: bool,
) -> Result<Option<Value>> {
    with_kuzu_connection(repo, |conn| {
        run_query_bridge(conn, search_query, context, goal, limit, include_content)
    })
}

pub fn try_run_context(
    name: Option<&str>,
    uid: Option<&str>,
    file_path: Option<&str>,
    repo: Option<&str>,
    include_content: bool,
) -> Result<Option<Value>> {
    with_kuzu_connection(repo, |conn| {
        run_context_bridge(conn, name, uid, file_path, include_content)
    })
}

pub fn try_run_impact(
    target: &str,
    direction: &str,
    repo: Option<&str>,
    max_depth: u32,
    include_tests: bool,
    relation_types: &[String],
    min_confidence: f32,
) -> Result<Option<Value>> {
    with_kuzu_connection(repo, |conn| {
        run_impact_bridge(
            conn,
            target,
            direction,
            max_depth,
            include_tests,
            relation_types,
            min_confidence,
        )
    })
}

fn run_query_bridge(
    conn: &Connection<'_>,
    search_query: &str,
    context: Option<&str>,
    goal: Option<&str>,
    limit: usize,
    include_content: bool,
) -> Result<Value> {
    let term_text = format!(
        "{} {} {}",
        search_query,
        context.unwrap_or_default(),
        goal.unwrap_or_default()
    );
    let terms = tokenize(&term_text);

    if terms.is_empty() {
        return Ok(json!({
            "processes": [],
            "process_symbols": [],
            "definitions": []
        }));
    }

    let safe_limit = limit.max(1);
    let search_limit = safe_limit * DEFAULT_MAX_SYMBOLS_PER_PROCESS * 3;
    let candidate_limit = (search_limit * 5).max(200);

    let term_filter = terms
        .iter()
        .map(|term| {
            let escaped = escape_cypher_string(term);
            format!(
                "toLower(n.name) CONTAINS '{escaped}' OR toLower(n.filePath) CONTAINS '{escaped}' OR toLower(labels(n)[0]) CONTAINS '{escaped}'"
            )
        })
        .collect::<Vec<_>>()
        .join(" OR ");

    let per_label_limit = (candidate_limit / SYMBOL_NODE_LABELS.len()).max(25);
    let rows = query_symbol_rows(conn, &term_filter, include_content, per_label_limit)?;

    #[derive(Debug)]
    struct QueryCandidate {
        score: f64,
        row: Map<String, Value>,
    }

    let mut candidates = Vec::<QueryCandidate>::new();
    for row in rows {
        let name = obj_get_str(&row, "name").unwrap_or_default();
        let file = obj_get_str(&row, "filePath").unwrap_or_default();
        let label = obj_get_str(&row, "type").unwrap_or_default();
        let score = score_symbol(&name, &file, &label, &terms);
        if score <= 0.0 {
            continue;
        }

        candidates.push(QueryCandidate { score, row });
    }

    candidates.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(Ordering::Equal));

    #[derive(Debug, Default)]
    struct ProcessBucket {
        id: String,
        summary: String,
        process_type: String,
        step_count: u64,
        total_score: f64,
        symbols: Vec<Value>,
    }

    let mut process_buckets = HashMap::<String, ProcessBucket>::new();
    let mut definitions = Vec::<Value>::new();

    for candidate in candidates.into_iter().take(search_limit) {
        let Some(symbol_id) = obj_get_str(&candidate.row, "id") else {
            continue;
        };

        let mut base = build_query_symbol_object(&candidate.row, include_content);
        if let Some(module) = lookup_symbol_module(conn, &symbol_id)? {
            base.insert("module".to_string(), Value::String(module));
        }

        let process_rows = query_objects(
            conn,
            &format!(
                "MATCH (n {{id: '{symbol_id}'}})-[r:CodeRelation {{type: 'STEP_IN_PROCESS'}}]->(p:Process)
                 RETURN p.id AS processId, p.label AS label, p.heuristicLabel AS heuristicLabel, p.processType AS processType, p.stepCount AS stepCount, r.step AS step",
                symbol_id = escape_cypher_string(&symbol_id)
            ),
        )?;

        if process_rows.is_empty() {
            definitions.push(Value::Object(base));
            continue;
        }

        for process_row in process_rows {
            let Some(process_id) = obj_get_str(&process_row, "processId") else {
                continue;
            };

            let step_index = obj_get_u64(&process_row, "step").unwrap_or(0);

            let mut symbol_entry = base.clone();
            symbol_entry.insert("process_id".to_string(), Value::String(process_id.clone()));
            symbol_entry.insert("step_index".to_string(), Value::Number(step_index.into()));

            let bucket = process_buckets.entry(process_id.clone()).or_default();
            if bucket.id.is_empty() {
                bucket.id = process_id.clone();
                bucket.summary = obj_get_str(&process_row, "heuristicLabel")
                    .filter(|s| !s.is_empty())
                    .or_else(|| obj_get_str(&process_row, "label").filter(|s| !s.is_empty()))
                    .unwrap_or_else(|| process_id.clone());
                bucket.process_type = obj_get_str(&process_row, "processType")
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| "heuristic".to_string());
                bucket.step_count = obj_get_u64(&process_row, "stepCount").unwrap_or(0);
            }
            bucket.total_score += candidate.score;
            bucket.symbols.push(Value::Object(symbol_entry));
        }
    }

    let mut ranked = process_buckets.into_values().collect::<Vec<_>>();
    ranked.sort_by(|a, b| {
        b.total_score
            .partial_cmp(&a.total_score)
            .unwrap_or(Ordering::Equal)
    });

    let selected = ranked.into_iter().take(safe_limit).collect::<Vec<_>>();
    let processes = selected
        .iter()
        .map(|process| {
            json!({
                "id": process.id,
                "summary": process.summary,
                "priority": round3(process.total_score),
                "symbol_count": process.symbols.len(),
                "process_type": process.process_type,
                "step_count": process.step_count,
            })
        })
        .collect::<Vec<_>>();

    let mut seen = HashSet::<String>::new();
    let mut process_symbols = Vec::<Value>::new();
    for process in &selected {
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

    Ok(json!({
        "processes": processes,
        "process_symbols": process_symbols,
        "definitions": definitions.into_iter().take(DEFAULT_DEFINITION_LIMIT).collect::<Vec<_>>(),
    }))
}

fn run_context_bridge(
    conn: &Connection<'_>,
    name: Option<&str>,
    uid: Option<&str>,
    file_path: Option<&str>,
    include_content: bool,
) -> Result<Value> {
    let Some(needle) = uid.or(name) else {
        return Ok(json!({ "error": "context requires either name or --uid" }));
    };

    let mut symbols = if let Some(uid) = uid {
        match lookup_symbol_by_id(conn, uid, include_content)? {
            Some(row) => vec![row],
            None => Vec::new(),
        }
    } else {
        let Some(name) = name else {
            return Ok(json!({ "error": "context requires either name or --uid" }));
        };
        let escaped = escape_cypher_string(name);
        let condition = format!("n.name = '{escaped}' OR n.id = '{escaped}'");
        let mut rows = query_symbol_rows(conn, &condition, include_content, 10)?;
        if let Some(file_filter) = file_path {
            rows.retain(|row| {
                obj_get_str(row, "filePath")
                    .map(|path| path.contains(file_filter))
                    .unwrap_or(false)
            });
        }
        rows
    };

    if symbols.is_empty() {
        return Ok(json!({
            "error": format!("Symbol '{needle}' not found")
        }));
    }

    if symbols.len() > 1 && uid.is_none() {
        let candidates = symbols
            .iter()
            .take(10)
            .map(|row| {
                json!({
                    "uid": obj_get_str(row, "id").unwrap_or_default(),
                    "name": obj_get_str(row, "name").unwrap_or_default(),
                    "kind": obj_get_str(row, "type").unwrap_or_default(),
                    "filePath": obj_get_str(row, "filePath").unwrap_or_default(),
                    "line": row.get("startLine").cloned().unwrap_or(Value::Null),
                })
            })
            .collect::<Vec<_>>();

        return Ok(json!({
            "status": "ambiguous",
            "message": format!(
                "Found {} symbols matching '{}'. Use --uid or --file to disambiguate.",
                symbols.len(),
                name.unwrap_or_default()
            ),
            "candidates": candidates,
        }));
    }

    let sym = symbols.pop().unwrap_or_else(|| unreachable!());
    let sym_id = obj_get_str(&sym, "id").unwrap_or_default();

    let mut incoming_rows = Vec::<Map<String, Value>>::new();
    let mut outgoing_rows = Vec::<Map<String, Value>>::new();
    let escaped_sym_id = escape_cypher_string(&sym_id);
    for label in SYMBOL_NODE_LABELS {
        let label_expr = symbol_label_expr(label);
        let incoming_query = format!(
            "MATCH (caller:{label_expr})-[r:CodeRelation]->(n {{id: '{escaped_sym_id}'}})
             WHERE r.type IN ['CALLS', 'IMPORTS', 'EXTENDS', 'IMPLEMENTS']
             RETURN toLower(r.type) AS relType, caller.id AS uid, caller.name AS name, caller.filePath AS filePath, '{label}' AS kind
             LIMIT 80"
        );
        if let Ok(mut rows) = query_objects(conn, &incoming_query) {
            incoming_rows.append(&mut rows);
        }

        let outgoing_query = format!(
            "MATCH (n {{id: '{escaped_sym_id}'}})-[r:CodeRelation]->(target:{label_expr})
             WHERE r.type IN ['CALLS', 'IMPORTS', 'EXTENDS', 'IMPLEMENTS']
             RETURN toLower(r.type) AS relType, target.id AS uid, target.name AS name, target.filePath AS filePath, '{label}' AS kind
             LIMIT 80"
        );
        if let Ok(mut rows) = query_objects(conn, &outgoing_query) {
            outgoing_rows.append(&mut rows);
        }
    }

    let process_rows = query_objects(
        conn,
        &format!(
            "MATCH (n {{id: '{escaped_sym_id}'}})-[r:CodeRelation {{type: 'STEP_IN_PROCESS'}}]->(p:Process)
             RETURN p.id AS processId, p.label AS label, p.heuristicLabel AS heuristicLabel, p.stepCount AS stepCount, r.step AS step"
        ),
    )?;

    let mut incoming = categorize_context_refs(incoming_rows);
    let mut outgoing = categorize_context_refs(outgoing_rows);

    for refs in incoming.values_mut() {
        dedupe_context_refs(refs);
        refs.truncate(30);
    }
    for refs in outgoing.values_mut() {
        dedupe_context_refs(refs);
        refs.truncate(30);
    }

    let processes = process_rows
        .into_iter()
        .map(|row| {
            let process_id = obj_get_str(&row, "processId").unwrap_or_default();
            let name = obj_get_str(&row, "heuristicLabel")
                .filter(|s| !s.is_empty())
                .or_else(|| obj_get_str(&row, "label").filter(|s| !s.is_empty()))
                .unwrap_or_else(|| process_id.clone());

            json!({
                "id": process_id,
                "name": name,
                "step_index": obj_get_u64(&row, "step").unwrap_or(0),
                "step_count": obj_get_u64(&row, "stepCount").unwrap_or(0),
            })
        })
        .collect::<Vec<_>>();

    let mut symbol_obj = Map::<String, Value>::new();
    symbol_obj.insert(
        "uid".to_string(),
        Value::String(obj_get_str(&sym, "id").unwrap_or_default()),
    );
    symbol_obj.insert(
        "name".to_string(),
        Value::String(obj_get_str(&sym, "name").unwrap_or_default()),
    );
    symbol_obj.insert(
        "kind".to_string(),
        Value::String(obj_get_str(&sym, "type").unwrap_or_default()),
    );
    symbol_obj.insert(
        "filePath".to_string(),
        Value::String(obj_get_str(&sym, "filePath").unwrap_or_default()),
    );

    if let Some(start) = obj_get_u64(&sym, "startLine") {
        let end = obj_get_u64(&sym, "endLine").unwrap_or(start);
        symbol_obj.insert("startLine".to_string(), Value::Number(start.into()));
        symbol_obj.insert("endLine".to_string(), Value::Number(end.into()));
    }

    if include_content
        && let Some(content) = obj_get_str(&sym, "content")
        && !content.is_empty()
    {
        symbol_obj.insert("content".to_string(), Value::String(content));
    }

    Ok(json!({
        "status": "found",
        "symbol": symbol_obj,
        "incoming": incoming,
        "outgoing": outgoing,
        "processes": processes,
    }))
}

fn run_impact_bridge(
    conn: &Connection<'_>,
    target: &str,
    direction: &str,
    max_depth: u32,
    include_tests: bool,
    relation_types: &[String],
    min_confidence: f32,
) -> Result<Value> {
    let escaped_target = escape_cypher_string(target);
    let mut target_rows =
        query_symbol_rows(conn, &format!("n.name = '{escaped_target}'"), false, 1)?;
    target_rows.sort_by(|a, b| {
        obj_get_str(a, "filePath")
            .unwrap_or_default()
            .cmp(&obj_get_str(b, "filePath").unwrap_or_default())
    });

    let Some(target_row) = target_rows.first() else {
        return Ok(json!({
            "error": format!("Target '{target}' not found")
        }));
    };

    let target_id = obj_get_str(target_row, "id").unwrap_or_default();
    if target_id.is_empty() {
        return Ok(json!({
            "error": format!("Target '{target}' not found")
        }));
    }

    let mut normalized_rel_types = relation_types
        .iter()
        .map(|t| t.trim().to_ascii_uppercase())
        .filter(|t| !t.is_empty())
        .collect::<Vec<_>>();
    if normalized_rel_types.is_empty() {
        normalized_rel_types = DEFAULT_RELATION_TYPES
            .iter()
            .map(|s| s.to_string())
            .collect();
    }

    let relation_filter = normalized_rel_types
        .iter()
        .map(|rel| format!("'{}'", escape_cypher_string(rel)))
        .collect::<Vec<_>>()
        .join(", ");
    let confidence_filter = if min_confidence > 0.0 {
        format!(" AND COALESCE(r.confidence, 1.0) >= {min_confidence}")
    } else {
        String::new()
    };

    #[derive(Debug, Clone)]
    struct ImpactItem {
        depth: u32,
        id: String,
        name: String,
        node_type: String,
        file_path: String,
        relation_type: String,
        confidence: f64,
    }

    let safe_depth = max_depth.max(1);
    let mut visited = HashSet::<String>::new();
    visited.insert(target_id.clone());

    let mut impacted = Vec::<ImpactItem>::new();
    let mut frontier = vec![target_id.clone()];
    let mut symbol_cache = HashMap::<String, Option<Map<String, Value>>>::new();

    for depth in 1..=safe_depth {
        if frontier.is_empty() {
            break;
        }

        let frontier_list = frontier
            .iter()
            .map(|id| format!("'{}'", escape_cypher_string(id)))
            .collect::<Vec<_>>()
            .join(", ");

        let traversal_query = match direction {
            "upstream" => format!(
                "MATCH (caller)-[r:CodeRelation]->(n)
                 WHERE n.id IN [{frontier_list}]
                   AND r.type IN [{relation_filter}]{confidence_filter}
                 RETURN caller.id AS id, r.type AS relType, r.confidence AS confidence",
            ),
            _ => format!(
                "MATCH (n)-[r:CodeRelation]->(callee)
                 WHERE n.id IN [{frontier_list}]
                   AND r.type IN [{relation_filter}]{confidence_filter}
                 RETURN callee.id AS id, r.type AS relType, r.confidence AS confidence",
            ),
        };

        let related_rows = query_objects(conn, &traversal_query)?;
        let mut next_frontier = Vec::<String>::new();

        for row in related_rows {
            let Some(id) = obj_get_str(&row, "id") else {
                continue;
            };
            if visited.contains(&id) {
                continue;
            }

            let detail = if let Some(cached) = symbol_cache.get(&id) {
                cached.clone()
            } else {
                let fetched = lookup_symbol_by_id(conn, &id, false)?;
                symbol_cache.insert(id.clone(), fetched.clone());
                fetched
            };

            let Some(detail) = detail else {
                continue;
            };

            let file_path = obj_get_str(&detail, "filePath").unwrap_or_default();
            if !include_tests && is_test_file_path(&file_path) {
                continue;
            }

            visited.insert(id.clone());
            next_frontier.push(id.clone());
            impacted.push(ImpactItem {
                depth,
                id,
                name: obj_get_str(&detail, "name").unwrap_or_default(),
                node_type: obj_get_str(&detail, "type").unwrap_or_default(),
                file_path,
                relation_type: obj_get_str(&row, "relType").unwrap_or_default(),
                confidence: obj_get_f64(&row, "confidence").unwrap_or(1.0),
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
        .map(|item| item.id.clone())
        .collect::<Vec<_>>();
    let direct_ids = impacted
        .iter()
        .filter(|item| item.depth == 1)
        .map(|item| item.id.clone())
        .collect::<Vec<_>>();

    let mut affected_processes = Vec::<Value>::new();
    let mut affected_modules = Vec::<Value>::new();

    if !impacted_ids.is_empty() {
        let impacted_list = ids_to_cypher_list(&impacted_ids);
        let direct_list = ids_to_cypher_list(&direct_ids);

        let process_rows = query_objects(
            conn,
            &format!(
                "MATCH (s)-[r:CodeRelation {{type: 'STEP_IN_PROCESS'}}]->(p:Process)
                 WHERE s.id IN [{impacted_list}]
                 RETURN p.id AS processId, p.label AS label, p.heuristicLabel AS heuristicLabel, COUNT(DISTINCT s.id) AS hits, MIN(r.step) AS minStep, p.stepCount AS stepCount
                 ORDER BY hits DESC
                 LIMIT 20"
            ),
        )?;
        affected_processes = process_rows
            .into_iter()
            .map(|row| {
                let process_id = obj_get_str(&row, "processId").unwrap_or_default();
                let process_name = obj_get_str(&row, "heuristicLabel")
                    .filter(|s| !s.is_empty())
                    .or_else(|| obj_get_str(&row, "label").filter(|s| !s.is_empty()))
                    .unwrap_or(process_id);

                json!({
                    "name": process_name,
                    "hits": obj_get_u64(&row, "hits").unwrap_or(0),
                    "broken_at_step": obj_get_u64(&row, "minStep").unwrap_or(0),
                    "step_count": obj_get_u64(&row, "stepCount").unwrap_or(0),
                })
            })
            .collect::<Vec<_>>();

        let module_rows = query_objects(
            conn,
            &format!(
                "MATCH (s)-[:CodeRelation {{type: 'MEMBER_OF'}}]->(c:Community)
                 WHERE s.id IN [{impacted_list}]
                 RETURN c.id AS moduleId, c.heuristicLabel AS heuristicLabel, c.label AS label, COUNT(DISTINCT s.id) AS hits
                 ORDER BY hits DESC
                 LIMIT 20"
            ),
        )?;

        let direct_module_rows = if direct_ids.is_empty() {
            Vec::new()
        } else {
            query_objects(
                conn,
                &format!(
                    "MATCH (s)-[:CodeRelation {{type: 'MEMBER_OF'}}]->(c:Community)
                     WHERE s.id IN [{direct_list}]
                     RETURN DISTINCT c.id AS moduleId, c.heuristicLabel AS heuristicLabel, c.label AS label"
                ),
            )?
        };

        let direct_modules = direct_module_rows
            .into_iter()
            .map(|row| module_name_from_row(&row))
            .collect::<HashSet<_>>();

        affected_modules = module_rows
            .into_iter()
            .map(|row| {
                let module_name = module_name_from_row(&row);
                json!({
                    "name": module_name,
                    "hits": obj_get_u64(&row, "hits").unwrap_or(0),
                    "impact": if direct_modules.contains(&module_name) { "direct" } else { "indirect" },
                })
            })
            .collect::<Vec<_>>();

        affected_modules.sort_by(|a, b| b["hits"].as_u64().cmp(&a["hits"].as_u64()));
    }

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

    Ok(json!({
        "target": {
            "id": obj_get_str(target_row, "id").unwrap_or_default(),
            "name": obj_get_str(target_row, "name").unwrap_or_default(),
            "type": obj_get_str(target_row, "type").unwrap_or_default(),
            "filePath": obj_get_str(target_row, "filePath").unwrap_or_default(),
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
    }))
}

fn with_kuzu_connection<T, F>(repo_hint: Option<&str>, operation: F) -> Result<Option<T>>
where
    F: FnOnce(&Connection<'_>) -> Result<T>,
{
    let repo_path = resolve_repo_path(repo_hint)?;
    let kuzu_path = get_storage_paths(&repo_path).storage_path.join("kuzu");

    if !kuzu_path.exists() {
        return Ok(None);
    }

    let db = match Database::new(&kuzu_path, SystemConfig::default().read_only(true)) {
        Ok(db) => db,
        Err(_) => return Ok(None),
    };
    let conn = match Connection::new(&db) {
        Ok(conn) => conn,
        Err(_) => return Ok(None),
    };

    let result = operation(&conn)?;
    Ok(Some(result))
}

fn query_objects(conn: &Connection<'_>, query: &str) -> Result<Vec<Map<String, Value>>> {
    let mut result = conn
        .query(query)
        .with_context(|| format!("native kuzu query failed: {}", truncate(query, 180)))?;
    let headers = result.get_column_names();

    let mut rows = Vec::<Map<String, Value>>::new();
    for row in result.by_ref() {
        rows.push(row_to_object(&headers, row));
    }

    Ok(rows)
}

fn row_to_object(headers: &[String], row: Vec<KuzuValue>) -> Map<String, Value> {
    let mut object = Map::<String, Value>::new();
    for (idx, value) in row.into_iter().enumerate() {
        let key = headers
            .get(idx)
            .cloned()
            .unwrap_or_else(|| format!("col_{idx}"));
        object.insert(key, kuzu_value_to_json(value));
    }
    object
}

fn build_query_symbol_object(
    row: &Map<String, Value>,
    include_content: bool,
) -> Map<String, Value> {
    let mut object = Map::<String, Value>::new();
    object.insert(
        "id".to_string(),
        Value::String(obj_get_str(row, "id").unwrap_or_default()),
    );
    object.insert(
        "name".to_string(),
        Value::String(obj_get_str(row, "name").unwrap_or_default()),
    );
    object.insert(
        "type".to_string(),
        Value::String(obj_get_str(row, "type").unwrap_or_default()),
    );
    object.insert(
        "filePath".to_string(),
        Value::String(obj_get_str(row, "filePath").unwrap_or_default()),
    );

    if let Some(start) = obj_get_u64(row, "startLine") {
        let end = obj_get_u64(row, "endLine").unwrap_or(start);
        object.insert("startLine".to_string(), Value::Number(start.into()));
        object.insert("endLine".to_string(), Value::Number(end.into()));
    }

    if include_content
        && let Some(content) = obj_get_str(row, "content")
        && !content.is_empty()
    {
        object.insert("content".to_string(), Value::String(content));
    }

    object
}

fn lookup_symbol_module(conn: &Connection<'_>, symbol_id: &str) -> Result<Option<String>> {
    let escaped_id = escape_cypher_string(symbol_id);
    let rows = query_objects(
        conn,
        &format!(
            "MATCH (n {{id: '{escaped_id}'}})-[:CodeRelation {{type: 'MEMBER_OF'}}]->(c:Community)
             RETURN c.id AS moduleId, c.heuristicLabel AS heuristicLabel, c.label AS label
             LIMIT 1"
        ),
    )?;

    let Some(row) = rows.first() else {
        return Ok(None);
    };

    let module = obj_get_str(row, "moduleId")
        .filter(|s| !s.is_empty())
        .or_else(|| obj_get_str(row, "heuristicLabel").filter(|s| !s.is_empty()))
        .or_else(|| obj_get_str(row, "label").filter(|s| !s.is_empty()));

    Ok(module)
}

fn module_name_from_row(row: &Map<String, Value>) -> String {
    obj_get_str(row, "moduleId")
        .filter(|s| !s.is_empty())
        .or_else(|| obj_get_str(row, "heuristicLabel").filter(|s| !s.is_empty()))
        .or_else(|| obj_get_str(row, "label").filter(|s| !s.is_empty()))
        .unwrap_or_else(|| "unknown".to_string())
}

fn categorize_context_refs(rows: Vec<Map<String, Value>>) -> BTreeMap<String, Vec<Value>> {
    let mut categories = BTreeMap::<String, Vec<Value>>::new();

    for row in rows {
        let rel_type = obj_get_str(&row, "relType")
            .map(|value| value.to_ascii_lowercase())
            .unwrap_or_default();
        if rel_type.is_empty() {
            continue;
        }

        categories.entry(rel_type).or_default().push(json!({
            "uid": obj_get_str(&row, "uid").unwrap_or_default(),
            "name": obj_get_str(&row, "name").unwrap_or_default(),
            "filePath": obj_get_str(&row, "filePath").unwrap_or_default(),
            "kind": obj_get_str(&row, "kind").unwrap_or_default(),
        }));
    }

    categories
}

fn dedupe_context_refs(refs: &mut Vec<Value>) {
    let mut seen = HashSet::<String>::new();
    refs.retain(|item| {
        let key = format!(
            "{}::{}::{}",
            item.get("uid").and_then(Value::as_str).unwrap_or_default(),
            item.get("kind").and_then(Value::as_str).unwrap_or_default(),
            item.get("filePath")
                .and_then(Value::as_str)
                .unwrap_or_default()
        );
        seen.insert(key)
    });
}

fn ids_to_cypher_list(ids: &[String]) -> String {
    ids.iter()
        .map(|id| format!("'{}'", escape_cypher_string(id)))
        .collect::<Vec<_>>()
        .join(", ")
}

fn query_symbol_rows(
    conn: &Connection<'_>,
    where_clause: &str,
    include_content: bool,
    limit_per_label: usize,
) -> Result<Vec<Map<String, Value>>> {
    let content_select = if include_content {
        ", n.content AS content"
    } else {
        ""
    };

    let mut rows = Vec::<Map<String, Value>>::new();
    for label in SYMBOL_NODE_LABELS {
        let label_expr = symbol_label_expr(label);
        let query = format!(
            "MATCH (n:{label_expr})
             WHERE {where_clause}
             RETURN n.id AS id, n.name AS name, '{label}' AS type, n.filePath AS filePath, n.startLine AS startLine, n.endLine AS endLine{content_select}
             LIMIT {limit_per_label}"
        );

        if let Ok(mut chunk) = query_objects(conn, &query) {
            rows.append(&mut chunk);
        }
    }

    Ok(rows)
}

fn lookup_symbol_by_id(
    conn: &Connection<'_>,
    symbol_id: &str,
    include_content: bool,
) -> Result<Option<Map<String, Value>>> {
    let escaped = escape_cypher_string(symbol_id);
    let mut rows = query_symbol_rows(conn, &format!("n.id = '{escaped}'"), include_content, 1)?;
    Ok(rows.pop())
}

fn symbol_label_expr(label: &str) -> String {
    match label {
        "Struct" | "Enum" | "Macro" | "Typedef" | "Union" | "Namespace" | "Trait" | "Impl"
        | "TypeAlias" | "Const" | "Static" | "Property" | "Record" | "Delegate" | "Annotation"
        | "Constructor" | "Template" | "Module" => format!("`{label}`"),
        _ => label.to_string(),
    }
}

fn tokenize(text: &str) -> Vec<String> {
    text.split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_' && ch != '$' && ch != '/')
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| s.len() >= 2)
        .collect::<Vec<_>>()
}

fn score_symbol(name: &str, file: &str, label: &str, terms: &[String]) -> f64 {
    let name = name.to_ascii_lowercase();
    let file = file.to_ascii_lowercase();
    let label = label.to_ascii_lowercase();

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

fn escape_cypher_string(raw: &str) -> String {
    raw.replace('\\', "\\\\").replace('\'', "''")
}

fn obj_get_str(row: &Map<String, Value>, key: &str) -> Option<String> {
    row.get(key).and_then(value_as_string)
}

fn obj_get_u64(row: &Map<String, Value>, key: &str) -> Option<u64> {
    row.get(key).and_then(value_as_u64)
}

fn obj_get_f64(row: &Map<String, Value>, key: &str) -> Option<f64> {
    row.get(key).and_then(value_as_f64)
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
            .or_else(|| v.as_i64().and_then(|x| u64::try_from(x).ok()))
            .or_else(|| {
                v.as_f64()
                    .and_then(|x| if x >= 0.0 { Some(x as u64) } else { None })
            }),
        Value::String(v) => v.parse::<u64>().ok(),
        Value::Bool(v) => Some(u64::from(*v)),
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}

fn value_as_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(v) => v.as_f64(),
        Value::String(v) => v.parse::<f64>().ok(),
        Value::Bool(v) => Some(if *v { 1.0 } else { 0.0 }),
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
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
        KuzuValue::Float(v) => serde_json::Number::from_f64(v as f64)
            .map(Value::Number)
            .unwrap_or_else(|| Value::String(v.to_string())),
        KuzuValue::Double(v) => serde_json::Number::from_f64(v)
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

fn render_markdown_table(headers: &[String], rows: &[Vec<Value>]) -> String {
    if headers.is_empty() {
        return String::new();
    }

    let mut lines = Vec::with_capacity(rows.len() + 2);
    let escaped_headers = headers
        .iter()
        .map(|header| escape_markdown_cell(header))
        .collect::<Vec<_>>();
    lines.push(format!("| {} |", escaped_headers.join(" | ")));
    lines.push(format!(
        "| {} |",
        headers
            .iter()
            .map(|_| "---")
            .collect::<Vec<_>>()
            .join(" | ")
    ));

    for row in rows {
        let mut cells = Vec::with_capacity(headers.len());
        for idx in 0..headers.len() {
            let rendered = row.get(idx).map(format_markdown_cell).unwrap_or_default();
            cells.push(rendered);
        }
        lines.push(format!("| {} |", cells.join(" | ")));
    }

    lines.join("\n")
}

fn format_markdown_cell(value: &Value) -> String {
    match value {
        Value::Null => String::new(),
        Value::String(s) => escape_markdown_cell(s),
        Value::Bool(v) => v.to_string(),
        Value::Number(v) => v.to_string(),
        Value::Array(_) | Value::Object(_) => {
            let text = serde_json::to_string(value).unwrap_or_else(|_| value.to_string());
            escape_markdown_cell(&text)
        }
    }
}

fn escape_markdown_cell(raw: &str) -> String {
    raw.replace('|', "\\|")
        .replace('\n', "<br>")
        .replace('\r', "")
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
            let names = entries
                .iter()
                .map(|entry| entry.name.clone())
                .collect::<Vec<_>>();
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
            let names = matches
                .iter()
                .map(|entry| entry.name.clone())
                .collect::<Vec<_>>();
            bail!(
                "Repository '{hint}' is ambiguous. Candidates: {}",
                names.join(", ")
            )
        }
    }
}
