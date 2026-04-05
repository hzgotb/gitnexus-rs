use anyhow::Result;

use crate::commands::delegate::run_ts_cli;

#[derive(Debug, Clone, Default)]
pub struct QueryOptions {
    pub repo: Option<String>,
    pub context: Option<String>,
    pub goal: Option<String>,
    pub limit: Option<u32>,
    pub content: bool,
}

pub fn run(search_query: &str, options: QueryOptions) -> Result<()> {
    let mut args = vec![search_query.to_string()];

    if let Some(repo) = options.repo {
        args.push("--repo".to_string());
        args.push(repo);
    }

    if let Some(context) = options.context {
        args.push("--context".to_string());
        args.push(context);
    }

    if let Some(goal) = options.goal {
        args.push("--goal".to_string());
        args.push(goal);
    }

    if let Some(limit) = options.limit {
        args.push("--limit".to_string());
        args.push(limit.to_string());
    }

    if options.content {
        args.push("--content".to_string());
    }

    run_ts_cli("query", &args)
}
