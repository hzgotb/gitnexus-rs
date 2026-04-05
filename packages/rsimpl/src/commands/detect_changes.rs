use anyhow::Result;

use crate::commands::delegate::run_ts_cli;

#[derive(Debug, Clone, Default)]
pub struct DetectChangesOptions {
    pub repo: Option<String>,
    pub scope: Option<String>,
    pub base_ref: Option<String>,
}

pub fn run(options: DetectChangesOptions) -> Result<()> {
    let mut args = Vec::<String>::new();

    if let Some(scope) = options.scope {
        args.push("--scope".to_string());
        args.push(scope);
    }

    if let Some(base_ref) = options.base_ref {
        args.push("--base-ref".to_string());
        args.push(base_ref);
    }

    if let Some(repo) = options.repo {
        args.push("--repo".to_string());
        args.push(repo);
    }

    run_ts_cli("detect_changes", &args)
}
