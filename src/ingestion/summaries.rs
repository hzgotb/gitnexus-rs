use std::collections::{BTreeMap, HashSet};

use super::{HeuristicCommunity, HeuristicProcess, StructureGraph, generate_id};

pub(super) fn build_process_summaries(graph: &StructureGraph) -> Vec<HeuristicProcess> {
    let mut out_neighbors = BTreeMap::<String, HashSet<String>>::new();
    let mut undirected = BTreeMap::<String, HashSet<String>>::new();
    let mut indegree = BTreeMap::<String, u64>::new();

    for rel in &graph.relationships {
        if rel.rel_type != "CALLS" {
            continue;
        }

        out_neighbors
            .entry(rel.source_id.clone())
            .or_default()
            .insert(rel.target_id.clone());
        indegree
            .entry(rel.target_id.clone())
            .and_modify(|d| *d += 1)
            .or_insert(1);
        indegree.entry(rel.source_id.clone()).or_insert(0);

        undirected
            .entry(rel.source_id.clone())
            .or_default()
            .insert(rel.target_id.clone());
        undirected
            .entry(rel.target_id.clone())
            .or_default()
            .insert(rel.source_id.clone());
    }

    if undirected.is_empty() {
        return Vec::new();
    }

    let mut visited = HashSet::<String>::new();
    let mut processes = Vec::<HeuristicProcess>::new();
    let mut process_index = 0usize;

    for node in undirected.keys() {
        if visited.contains(node) {
            continue;
        }

        let mut component_nodes = HashSet::<String>::new();
        let mut stack = vec![node.clone()];
        while let Some(cur) = stack.pop() {
            if !visited.insert(cur.clone()) {
                continue;
            }
            component_nodes.insert(cur.clone());
            if let Some(next) = undirected.get(&cur) {
                for n in next {
                    if !visited.contains(n) {
                        stack.push(n.clone());
                    }
                }
            }
        }

        let mut entry_candidates: Vec<String> = component_nodes
            .iter()
            .filter(|id| indegree.get(*id).copied().unwrap_or(0) == 0)
            .cloned()
            .collect();
        if entry_candidates.is_empty() {
            entry_candidates = component_nodes.iter().cloned().collect();
        }
        entry_candidates.sort();
        let entry = entry_candidates
            .into_iter()
            .next()
            .unwrap_or_else(|| node.clone());

        let (symbol_ids, max_depth) = traverse_process(&entry, &component_nodes, &out_neighbors);
        process_index += 1;
        processes.push(HeuristicProcess {
            id: format!("process_{process_index}"),
            entry_symbol_id: entry,
            symbol_count: symbol_ids.len() as u64,
            step_count: max_depth.saturating_add(1),
            symbols: symbol_ids,
        });
    }

    processes
}

fn traverse_process(
    entry: &str,
    component_nodes: &HashSet<String>,
    out_neighbors: &BTreeMap<String, HashSet<String>>,
) -> (Vec<String>, u64) {
    let mut visited = HashSet::<String>::new();
    let mut stack = vec![(entry.to_string(), 0u64)];
    let mut max_depth = 0u64;

    while let Some((cur, depth)) = stack.pop() {
        if !visited.insert(cur.clone()) {
            continue;
        }
        max_depth = max_depth.max(depth);
        if let Some(next) = out_neighbors.get(&cur) {
            for n in next {
                if component_nodes.contains(n) && !visited.contains(n) {
                    stack.push((n.clone(), depth + 1));
                }
            }
        }
    }

    for node in component_nodes {
        if !visited.contains(node) {
            visited.insert(node.clone());
        }
    }

    let mut symbols = visited.into_iter().collect::<Vec<_>>();
    symbols.sort();
    (symbols, max_depth)
}

pub(super) fn build_community_summaries(graph: &StructureGraph) -> Vec<HeuristicCommunity> {
    let mut node_by_id = BTreeMap::<String, _>::new();
    for node in &graph.nodes {
        node_by_id.insert(node.id.clone(), node);
    }

    let mut adjacency = BTreeMap::<String, HashSet<String>>::new();
    let mut active_files = HashSet::<String>::new();

    for rel in &graph.relationships {
        match rel.rel_type.as_str() {
            "IMPORTS" => {
                if is_file_id(&rel.source_id) && is_file_id(&rel.target_id) {
                    connect_undirected(&mut adjacency, &rel.source_id, &rel.target_id);
                    active_files.insert(rel.source_id.clone());
                    active_files.insert(rel.target_id.clone());
                }
            }
            "CALLS" | "EXTENDS" | "IMPLEMENTS" => {
                let Some(source_node) = node_by_id.get(&rel.source_id) else {
                    continue;
                };
                let Some(target_node) = node_by_id.get(&rel.target_id) else {
                    continue;
                };

                let source_file_id = generate_id("File", &source_node.file_path);
                let target_file_id = generate_id("File", &target_node.file_path);
                if source_file_id == target_file_id {
                    active_files.insert(source_file_id);
                    continue;
                }

                connect_undirected(&mut adjacency, &source_file_id, &target_file_id);
                active_files.insert(source_file_id);
                active_files.insert(target_file_id);
            }
            _ => {}
        }
    }

    if active_files.is_empty() {
        return build_fallback_communities(graph);
    }

    let mut visited = HashSet::<String>::new();
    let mut communities = Vec::<HeuristicCommunity>::new();
    let mut community_index = 0usize;
    for file in &active_files {
        if visited.contains(file) {
            continue;
        }

        let mut files = Vec::<String>::new();
        let mut stack = vec![file.clone()];
        while let Some(cur) = stack.pop() {
            if !visited.insert(cur.clone()) {
                continue;
            }
            files.push(strip_file_node_prefix(&cur).to_string());
            if let Some(next) = adjacency.get(&cur) {
                for n in next {
                    if !visited.contains(n) {
                        stack.push(n.clone());
                    }
                }
            }
        }

        files.sort();
        community_index += 1;
        communities.push(HeuristicCommunity {
            id: format!("community_{community_index}"),
            file_count: files.len() as u64,
            files,
        });
    }

    communities
}

pub(super) fn build_fallback_communities(graph: &StructureGraph) -> Vec<HeuristicCommunity> {
    let mut groups = BTreeMap::<String, Vec<String>>::new();
    for node in &graph.nodes {
        if node.label != "File" {
            continue;
        }

        let group = node
            .file_path
            .split('/')
            .next()
            .unwrap_or_default()
            .trim()
            .to_string();
        if group.is_empty() {
            continue;
        }
        groups
            .entry(group)
            .or_default()
            .push(node.file_path.clone());
    }

    let mut communities = Vec::<HeuristicCommunity>::new();
    for (idx, (_group, mut files)) in groups.into_iter().enumerate() {
        files.sort();
        communities.push(HeuristicCommunity {
            id: format!("community_{}", idx + 1),
            file_count: files.len() as u64,
            files,
        });
    }

    communities
}

fn is_file_id(node_id: &str) -> bool {
    node_id.starts_with("File:")
}

fn strip_file_node_prefix(node_id: &str) -> &str {
    node_id.strip_prefix("File:").unwrap_or(node_id)
}

fn connect_undirected(adjacency: &mut BTreeMap<String, HashSet<String>>, a: &str, b: &str) {
    adjacency
        .entry(a.to_string())
        .or_default()
        .insert(b.to_string());
    adjacency
        .entry(b.to_string())
        .or_default()
        .insert(a.to_string());
}
