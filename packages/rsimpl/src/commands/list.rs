use anyhow::Result;

use crate::storage::repo_manager::list_registered_repos;

pub fn run() -> Result<()> {
    let entries = list_registered_repos(true)?;

    if entries.is_empty() {
        println!("No indexed repositories found.");
        println!("Run `gitnexus analyze` in a git repo to index it.");
        return Ok(());
    }

    println!("\n  Indexed Repositories ({})\n", entries.len());

    for entry in entries {
        let stats = entry.stats.unwrap_or_default();
        let commit_short = entry.last_commit.chars().take(7).collect::<String>();

        println!("  {}", entry.name);
        println!("    Path:    {}", entry.path);
        println!("    Indexed: {}", entry.indexed_at);
        println!(
            "    Commit:  {}",
            if commit_short.is_empty() {
                "unknown".to_string()
            } else {
                commit_short
            }
        );
        println!(
            "    Stats:   {} files, {} symbols, {} edges",
            stats.files.unwrap_or(0),
            stats.nodes.unwrap_or(0),
            stats.edges.unwrap_or(0)
        );

        if let Some(communities) = stats.communities {
            println!("    Clusters:   {communities}");
        }

        if let Some(processes) = stats.processes {
            println!("    Processes:  {processes}");
        }

        println!();
    }

    Ok(())
}
