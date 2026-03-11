use anyhow::Result;

use crate::commands::delegate::run_ts_cli;

#[derive(Debug, Clone)]
pub struct ImpactOptions {
    pub direction: String,
    pub repo: Option<String>,
    pub depth: Option<u32>,
    pub include_tests: bool,
}

pub fn run(target: &str, options: ImpactOptions) -> Result<()> {
    let mut args = vec![
        target.to_string(),
        "--direction".to_string(),
        options.direction,
    ];

    if let Some(repo) = options.repo {
        args.push("--repo".to_string());
        args.push(repo);
    }

    if let Some(depth) = options.depth {
        args.push("--depth".to_string());
        args.push(depth.to_string());
    }

    if options.include_tests {
        args.push("--include-tests".to_string());
    }

    run_ts_cli("impact", &args)
}
