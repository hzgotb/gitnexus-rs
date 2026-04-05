use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::Utc;

use crate::git::{get_git_root, is_git_repo};
use crate::ingestion::{IngestionResult, StructureNode, run_ingestion_pipeline};
use crate::storage::repo_manager::{get_storage_paths, load_meta};

#[derive(Debug, Clone, Default)]
pub struct WikiOptions {
    pub force: bool,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub concurrency: Option<u16>,
    pub gist: bool,
}

pub fn run(path: Option<&Path>, options: WikiOptions) -> Result<()> {
    println!("\n  GitNexus Wiki Generator (Rust baseline)\n");

    let repo_path = match resolve_repo_path(path) {
        Ok(path) => path,
        Err(_) => {
            println!("  Not inside a git repository\n");
            return Ok(());
        }
    };

    if !is_git_repo(&repo_path) {
        println!("  Not a git repository\n");
        return Ok(());
    }

    let storage = get_storage_paths(&repo_path);
    let wiki_dir = storage.storage_path.join(WIKI_DIRNAME);

    if wiki_dir.exists() {
        if options.force {
            std::fs::remove_dir_all(&wiki_dir).with_context(|| {
                format!(
                    "failed to remove existing wiki dir {}",
                    wiki_dir.to_string_lossy()
                )
            })?;
        } else {
            println!(
                "  Wiki already exists: {}\n  Use --force to regenerate.\n",
                wiki_dir.to_string_lossy()
            );
            return Ok(());
        }
    }

    let graph = load_or_build_graph(&repo_path, &storage.storage_path)?;
    std::fs::create_dir_all(&wiki_dir)
        .with_context(|| format!("failed to create wiki dir {}", wiki_dir.to_string_lossy()))?;

    generate_wiki_files(&repo_path, &storage.storage_path, &wiki_dir, &graph)?;

    if options.model.is_some()
        || options.base_url.is_some()
        || options.api_key.is_some()
        || options.concurrency.is_some()
        || options.gist
    {
        println!(
            "  Note: model/base-url/api-key/concurrency/gist options are not yet implemented in Rust wiki baseline."
        );
    }

    println!("  Wiki generated successfully");
    println!("  Output: {}", wiki_dir.to_string_lossy());
    println!("  Files: README.md, OVERVIEW.md, MODULES.md, PROCESSES.md, ARCHITECTURE.md\n");
    Ok(())
}

const WIKI_DIRNAME: &str = "wiki";
const GRAPH_CACHE_FILENAME: &str = "graph.json";

fn resolve_repo_path(input_path: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = input_path {
        return absolutize(path);
    }

    let cwd = std::env::current_dir().context("failed to read current working directory")?;
    match get_git_root(&cwd) {
        Some(root) => Ok(root),
        None => bail!("not inside a git repository"),
    }
}

fn absolutize(path: &Path) -> Result<PathBuf> {
    if path.is_absolute() {
        return Ok(path.to_path_buf());
    }
    let cwd = std::env::current_dir().context("failed to read current working directory")?;
    Ok(cwd.join(path))
}

fn load_or_build_graph(repo_path: &Path, storage_path: &Path) -> Result<IngestionResult> {
    let graph_path = storage_path.join(GRAPH_CACHE_FILENAME);
    if graph_path.exists() {
        let raw = std::fs::read_to_string(&graph_path)
            .with_context(|| format!("failed to read {}", graph_path.to_string_lossy()))?;
        return serde_json::from_str::<IngestionResult>(&raw)
            .with_context(|| format!("failed to parse {}", graph_path.to_string_lossy()));
    }

    let graph = run_ingestion_pipeline(repo_path)?;
    std::fs::create_dir_all(storage_path).with_context(|| {
        format!(
            "failed to create storage directory {}",
            storage_path.to_string_lossy()
        )
    })?;
    let content = serde_json::to_string_pretty(&graph)?;
    std::fs::write(&graph_path, content)
        .with_context(|| format!("failed to write {}", graph_path.to_string_lossy()))?;
    Ok(graph)
}

fn generate_wiki_files(
    repo_path: &Path,
    storage_path: &Path,
    wiki_dir: &Path,
    graph: &IngestionResult,
) -> Result<()> {
    let generated_at = Utc::now().to_rfc3339();
    let project_name = repo_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("project");

    let node_map = graph
        .graph
        .nodes
        .iter()
        .cloned()
        .map(|n| (n.id.clone(), n))
        .collect::<HashMap<_, _>>();
    let module_by_file = build_file_module_map(graph);

    let overview = render_overview(project_name, repo_path, storage_path, &generated_at, graph)?;
    let modules = render_modules(graph);
    let processes = render_processes(graph, &node_map);
    let architecture = render_architecture(graph, &node_map, &module_by_file);
    let index = render_index(project_name, &generated_at);

    write_wiki_file(wiki_dir, "README.md", &index)?;
    write_wiki_file(wiki_dir, "OVERVIEW.md", &overview)?;
    write_wiki_file(wiki_dir, "MODULES.md", &modules)?;
    write_wiki_file(wiki_dir, "PROCESSES.md", &processes)?;
    write_wiki_file(wiki_dir, "ARCHITECTURE.md", &architecture)?;
    Ok(())
}

fn write_wiki_file(wiki_dir: &Path, filename: &str, content: &str) -> Result<()> {
    let path = wiki_dir.join(filename);
    std::fs::write(&path, content)
        .with_context(|| format!("failed to write {}", path.to_string_lossy()))?;
    Ok(())
}

fn render_index(project_name: &str, generated_at: &str) -> String {
    [
        &format!("# {project_name} Wiki"),
        "",
        &format!("Generated at: `{generated_at}`"),
        "",
        "## Contents",
        "",
        "- [Overview](./OVERVIEW.md)",
        "- [Modules](./MODULES.md)",
        "- [Processes](./PROCESSES.md)",
        "- [Architecture](./ARCHITECTURE.md)",
        "",
        "This wiki is generated by the Rust-native baseline `gitnexus wiki` implementation.",
    ]
    .join("\n")
}

fn render_overview(
    project_name: &str,
    repo_path: &Path,
    storage_path: &Path,
    generated_at: &str,
    graph: &IngestionResult,
) -> Result<String> {
    let meta = load_meta(storage_path)?;
    let mut lines = Vec::<String>::new();
    lines.push(format!("# {project_name} - Overview"));
    lines.push(String::new());
    lines.push(format!("Generated at: `{generated_at}`"));
    lines.push(format!(
        "Repository path: `{}`",
        repo_path.to_string_lossy()
    ));
    lines.push(String::new());
    lines.push("## Stats".to_string());
    lines.push(String::new());
    lines.push(format!("- Files: {}", graph.stats.total_files));
    lines.push(format!(
        "- Parseable files: {}",
        graph.stats.parseable_files
    ));
    lines.push(format!("- Symbols: {}", graph.stats.symbols));
    lines.push(format!("- Nodes: {}", graph.stats.nodes));
    lines.push(format!("- Edges: {}", graph.stats.edges));
    lines.push(format!("- Communities: {}", graph.stats.communities));
    lines.push(format!("- Processes: {}", graph.stats.processes));
    if graph.stats.skipped_large_files > 0 {
        lines.push(format!(
            "- Skipped large files (>512KB): {}",
            graph.stats.skipped_large_files
        ));
    }

    if let Some(meta) = meta {
        lines.push(String::new());
        lines.push("## Index Metadata".to_string());
        lines.push(String::new());
        lines.push(format!("- Indexed at: `{}`", meta.indexed_at));
        lines.push(format!(
            "- Last commit: `{}`",
            short_commit(&meta.last_commit)
        ));
    }

    if !graph.stats.language_breakdown.is_empty() {
        lines.push(String::new());
        lines.push("## Languages".to_string());
        lines.push(String::new());
        for (language, count) in &graph.stats.language_breakdown {
            lines.push(format!("- {language}: {count}"));
        }
    }

    Ok(lines.join("\n"))
}

fn render_modules(graph: &IngestionResult) -> String {
    let mut communities = graph.communities.clone();
    communities.sort_by(|a, b| b.file_count.cmp(&a.file_count));

    let mut lines = Vec::<String>::new();
    lines.push("# Modules".to_string());
    lines.push(String::new());

    if communities.is_empty() {
        lines.push("No communities detected.".to_string());
        return lines.join("\n");
    }

    for community in communities {
        lines.push(format!("## {}", community.id));
        lines.push(String::new());
        lines.push(format!("- Files: {}", community.file_count));
        lines.push(String::new());
        lines.push("Sample files:".to_string());
        for file in community.files.iter().take(12) {
            lines.push(format!("- `{file}`"));
        }
        lines.push(String::new());
    }

    lines.join("\n")
}

fn render_processes(graph: &IngestionResult, node_map: &HashMap<String, StructureNode>) -> String {
    let mut processes = graph.processes.clone();
    processes.sort_by(|a, b| b.step_count.cmp(&a.step_count));

    let mut lines = Vec::<String>::new();
    lines.push("# Processes".to_string());
    lines.push(String::new());

    if processes.is_empty() {
        lines.push("No processes detected.".to_string());
        return lines.join("\n");
    }

    for process in processes {
        lines.push(format!("## {}", process.id));
        lines.push(String::new());
        lines.push(format!("- Entry symbol: `{}`", process.entry_symbol_id));
        lines.push(format!("- Symbols: {}", process.symbol_count));
        lines.push(format!("- Steps: {}", process.step_count));
        lines.push(String::new());
        lines.push("Trace preview:".to_string());

        for (idx, symbol_id) in process.symbols.iter().take(20).enumerate() {
            let display = node_map
                .get(symbol_id)
                .map(|node| format!("{} ({})", node.name, node.file_path))
                .unwrap_or_else(|| symbol_id.clone());
            lines.push(format!("{}. {}", idx + 1, display));
        }

        if process.symbols.len() > 20 {
            lines.push(format!(
                "... and {} more symbols",
                process.symbols.len() - 20
            ));
        }
        lines.push(String::new());
    }

    lines.join("\n")
}

fn render_architecture(
    graph: &IngestionResult,
    node_map: &HashMap<String, StructureNode>,
    module_by_file: &HashMap<String, String>,
) -> String {
    let mut edge_counts = BTreeMap::<(String, String), u64>::new();

    for rel in &graph.graph.relationships {
        if !matches!(
            rel.rel_type.as_str(),
            "CALLS" | "IMPORTS" | "EXTENDS" | "IMPLEMENTS"
        ) {
            continue;
        }

        let source_file = resolve_rel_endpoint_file_path(&rel.source_id, node_map);
        let target_file = resolve_rel_endpoint_file_path(&rel.target_id, node_map);
        let (Some(source_file), Some(target_file)) = (source_file, target_file) else {
            continue;
        };

        let source_module = module_by_file
            .get(&source_file)
            .cloned()
            .unwrap_or_else(|| "unknown".to_string());
        let target_module = module_by_file
            .get(&target_file)
            .cloned()
            .unwrap_or_else(|| "unknown".to_string());
        if source_module == target_module {
            continue;
        }

        *edge_counts
            .entry((source_module, target_module))
            .or_insert(0) += 1;
    }

    let mut lines = Vec::<String>::new();
    lines.push("# Architecture".to_string());
    lines.push(String::new());
    lines.push("## Module Dependency Graph".to_string());
    lines.push(String::new());
    lines.push("```mermaid".to_string());
    lines.push("graph LR".to_string());

    if edge_counts.is_empty() {
        lines.push("  A[\"No cross-module edges\"]".to_string());
    } else {
        let mut module_index = BTreeMap::<String, usize>::new();
        for ((source, target), _) in &edge_counts {
            if !module_index.contains_key(source) {
                let next = module_index.len() + 1;
                module_index.insert(source.clone(), next);
            }
            if !module_index.contains_key(target) {
                let next = module_index.len() + 1;
                module_index.insert(target.clone(), next);
            }
        }

        for (name, idx) in &module_index {
            lines.push(format!("  M{idx}[\"{name}\"]"));
        }
        for ((source, target), count) in &edge_counts {
            let src = module_index.get(source).copied().unwrap_or(0);
            let dst = module_index.get(target).copied().unwrap_or(0);
            if src == 0 || dst == 0 {
                continue;
            }
            lines.push(format!("  M{src} -->|{count}| M{dst}"));
        }
    }

    lines.push("```".to_string());
    lines.push(String::new());
    lines.push("## Top Cross-Module Edges".to_string());
    lines.push(String::new());

    if edge_counts.is_empty() {
        lines.push("- No cross-module edges found.".to_string());
    } else {
        let mut ranked = edge_counts.into_iter().collect::<Vec<_>>();
        ranked.sort_by(|a, b| b.1.cmp(&a.1));
        for ((source, target), count) in ranked.into_iter().take(20) {
            lines.push(format!("- `{source}` -> `{target}`: {count} edges"));
        }
    }

    lines.join("\n")
}

fn resolve_rel_endpoint_file_path(
    endpoint_id: &str,
    node_map: &HashMap<String, StructureNode>,
) -> Option<String> {
    if let Some(path) = endpoint_id.strip_prefix("File:") {
        return Some(path.to_string());
    }
    node_map.get(endpoint_id).map(|node| node.file_path.clone())
}

fn build_file_module_map(graph: &IngestionResult) -> HashMap<String, String> {
    let mut out = HashMap::<String, String>::new();
    for community in &graph.communities {
        for file in &community.files {
            out.entry(file.clone())
                .or_insert_with(|| community.id.clone());
        }
    }
    out
}

fn short_commit(commit: &str) -> String {
    let short = commit.chars().take(7).collect::<String>();
    if short.is_empty() {
        "unknown".to_string()
    } else {
        short
    }
}
