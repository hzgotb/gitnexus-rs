use anyhow::Result;

use crate::commands::delegate::run_ts_cli;

#[derive(Debug, Clone, Default)]
pub struct CypherOptions {
    pub repo: Option<String>,
}

pub fn run(query: &str, options: CypherOptions) -> Result<()> {
    let mut args = vec![query.to_string()];

    if let Some(repo) = options.repo {
        args.push("--repo".to_string());
        args.push(repo);
    }

    run_ts_cli("cypher", &args)
}
