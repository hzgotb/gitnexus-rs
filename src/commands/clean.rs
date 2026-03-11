use std::path::Path;

use anyhow::Result;

use crate::storage::repo_manager::{find_repo, list_registered_repos, unregister_repo};

#[derive(Debug, Clone, Copy)]
pub struct CleanOptions {
    pub force: bool,
    pub all: bool,
}

pub fn run(options: CleanOptions) -> Result<()> {
    if options.all {
        return clean_all(options.force);
    }

    let cwd = std::env::current_dir()?;
    let repo = match find_repo(&cwd)? {
        Some(repo) => repo,
        None => {
            println!("No indexed repository found in this directory.");
            return Ok(());
        }
    };

    let repo_name = repo
        .repo_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("unknown");

    if !options.force {
        println!("This will delete the GitNexus index for: {repo_name}");
        println!("   Path: {}", repo.storage_path.to_string_lossy());
        println!("\nRun with --force to confirm deletion.");
        return Ok(());
    }

    remove_index_dir(&repo.storage_path)?;
    unregister_repo(&repo.repo_path)?;
    println!("Deleted: {}", repo.storage_path.to_string_lossy());
    Ok(())
}

fn clean_all(force: bool) -> Result<()> {
    let entries = list_registered_repos(false)?;
    if entries.is_empty() {
        println!("No indexed repositories found.");
        return Ok(());
    }

    if !force {
        println!(
            "This will delete GitNexus indexes for {} repo(s):",
            entries.len()
        );
        for entry in &entries {
            println!("  - {} ({})", entry.name, entry.path);
        }
        println!("\nRun with --force to confirm deletion.");
        return Ok(());
    }

    for entry in entries {
        let storage_path = Path::new(&entry.storage_path);
        remove_index_dir(storage_path)?;
        unregister_repo(Path::new(&entry.path))?;
        println!("Deleted: {} ({})", entry.name, entry.storage_path);
    }

    Ok(())
}

fn remove_index_dir(path: &Path) -> Result<()> {
    if path.exists() {
        std::fs::remove_dir_all(path)?;
    }
    Ok(())
}
