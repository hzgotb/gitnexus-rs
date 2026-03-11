use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use chrono::Utc;

use crate::git::{get_current_commit, get_git_root, is_git_repo};
use crate::ingestion::{run_ingestion_pipeline, save_ingestion_report};
use crate::storage::repo_manager::{
    RepoMeta, RepoStats, add_to_gitignore, get_storage_paths, load_meta, register_repo, save_meta,
};

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

    if let Some(existing) = existing_meta {
        if !options.force && existing.last_commit == current_commit {
            println!("  Already up to date\n");
            return Ok(());
        }
    }

    let ingestion = run_ingestion_pipeline(&repo_path)?;
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

    save_meta(&storage.storage_path, &meta)?;
    save_ingestion_report(&storage.storage_path, &ingestion)?;
    register_repo(&repo_path, &meta)?;
    add_to_gitignore(&repo_path)?;

    if options.embeddings {
        println!("  Note: --embeddings is not implemented in Rust preview yet.");
    }

    println!("  Repository indexed successfully (structure + parsing ingestion)");
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
    println!();

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
