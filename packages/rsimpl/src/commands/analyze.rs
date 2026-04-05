use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::Utc;

use crate::git::{get_current_commit, get_git_root, is_git_repo};
use crate::ingestion::{run_ingestion_pipeline, save_ingestion_report};
use crate::storage::kuzu_store::rebuild_from_ingestion;
use crate::storage::repo_manager::{
    RepoMeta, RepoStats, add_to_gitignore, get_storage_paths, load_meta, register_repo, save_meta,
};

const GRAPH_CACHE_FILENAME: &str = "graph.json";

#[derive(Debug, Clone, Copy)]
pub struct AnalyzeOptions {
    pub force: bool,
    pub embeddings: bool,
}

pub fn run(input_path: Option<&Path>, options: AnalyzeOptions) -> Result<()> {
    println!("\n  GitNexus Analyzer (Rust preview)\n");

    let repo_path = match resolve_repo_path(input_path) {
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
    let current_commit = get_current_commit(&repo_path);
    let existing_meta = load_meta(&storage.storage_path)?;
    let has_query_ready_artifacts = storage.kuzu_path.exists();

    if let Some(existing) = existing_meta {
        if !options.force && existing.last_commit == current_commit && has_query_ready_artifacts {
            println!("  Already up to date\n");
            return Ok(());
        }
    }

    let ingestion = run_ingestion_pipeline(&repo_path)?;
    let kuzu_report = rebuild_from_ingestion(&repo_path, &storage, &ingestion)?;
    let indexed_at = Utc::now().to_rfc3339();

    let meta = RepoMeta {
        repo_path: repo_path.to_string_lossy().to_string(),
        last_commit: current_commit,
        indexed_at: indexed_at.clone(),
        stats: Some(RepoStats {
            files: Some(ingestion.stats.total_files),
            nodes: Some(ingestion.stats.nodes),
            edges: Some(ingestion.stats.edges),
            communities: Some(ingestion.stats.communities),
            processes: Some(ingestion.stats.processes),
            embeddings: Some(0),
        }),
    };

    save_graph_cache(&storage.storage_path, &ingestion)?;
    save_meta(&storage.storage_path, &meta)?;
    save_ingestion_report(&storage.storage_path, &ingestion)?;
    register_repo(&repo_path, &meta)?;
    add_to_gitignore(&repo_path)?;

    if options.embeddings {
        println!("  Note: --embeddings is not implemented in Rust preview yet.");
    }

    println!("  Repository indexed successfully (ingestion + kuzu materialization)");
    println!("  Path: {}", repo_path.to_string_lossy());
    println!("  Indexed at: {indexed_at}");
    println!(
        "  Stats: {} files, {} nodes, {} edges",
        ingestion.stats.total_files, ingestion.stats.nodes, ingestion.stats.edges
    );
    println!(
        "  Structure: {} communities, {} processes",
        ingestion.stats.communities, ingestion.stats.processes
    );
    println!("  Extracted symbols: {}", ingestion.stats.symbols);
    println!("  Parseable files: {}", ingestion.stats.parseable_files);
    if ingestion.stats.skipped_large_files > 0 {
        println!(
            "  Skipped large files (>512KB): {}",
            ingestion.stats.skipped_large_files
        );
    }
    if !ingestion.stats.language_breakdown.is_empty() {
        println!("  Languages:");
        for (language, count) in &ingestion.stats.language_breakdown {
            println!("    - {language}: {count}");
        }
    }
    println!(
        "  Kuzu: {} nodes, {} edges -> {}",
        kuzu_report.indexed_nodes,
        kuzu_report.indexed_edges,
        storage.kuzu_path.to_string_lossy()
    );
    println!("  FTS indexes: {}", kuzu_report.fts_indexes);
    if !kuzu_report.fts_warnings.is_empty() {
        println!("  FTS warnings:");
        for warning in &kuzu_report.fts_warnings {
            println!("    - {warning}");
        }
    }
    println!();

    Ok(())
}

fn save_graph_cache(
    storage_path: &Path,
    ingestion: &crate::ingestion::IngestionResult,
) -> Result<()> {
    std::fs::create_dir_all(storage_path).with_context(|| {
        format!(
            "failed to create storage directory {}",
            storage_path.to_string_lossy()
        )
    })?;

    let graph_path = storage_path.join(GRAPH_CACHE_FILENAME);
    let content = serde_json::to_string_pretty(ingestion)?;
    std::fs::write(&graph_path, content)
        .with_context(|| format!("failed to write {}", graph_path.to_string_lossy()))?;
    Ok(())
}

fn resolve_repo_path(input_path: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = input_path {
        return Ok(absolutize(path)?);
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
