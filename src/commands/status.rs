use anyhow::Result;

use crate::git::{get_current_commit, is_git_repo};
use crate::storage::repo_manager::find_repo;

pub fn run() -> Result<()> {
    let cwd = std::env::current_dir()?;

    if !is_git_repo(&cwd) {
        println!("Not a git repository.");
        return Ok(());
    }

    let repo = match find_repo(&cwd)? {
        Some(repo) => repo,
        None => {
            println!("Repository not indexed.");
            println!("Run: gitnexus analyze");
            return Ok(());
        }
    };

    let current_commit = get_current_commit(&repo.repo_path);
    let indexed_commit = repo.meta.last_commit.clone();
    let up_to_date = !current_commit.is_empty() && current_commit == indexed_commit;

    println!("Repository: {}", repo.repo_path.to_string_lossy());
    println!("Indexed: {}", repo.meta.indexed_at);
    println!("Indexed commit: {}", short_commit(&indexed_commit));
    println!("Current commit: {}", short_commit(&current_commit));
    println!(
        "Status: {}",
        if up_to_date {
            "up-to-date"
        } else {
            "stale (re-run gitnexus analyze)"
        }
    );

    Ok(())
}

fn short_commit(commit: &str) -> String {
    let short = commit.chars().take(7).collect::<String>();
    if short.is_empty() {
        "unknown".to_string()
    } else {
        short
    }
}
