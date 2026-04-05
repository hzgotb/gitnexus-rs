use anyhow::Result;

use crate::commands::delegate::run_ts_cli;

#[derive(Debug, Clone)]
pub struct ImpactOptions {
    pub direction: String,
    pub repo: Option<String>,
    pub depth: Option<u32>,
    pub include_tests: bool,
    pub relation_types: Option<Vec<String>>,
    pub min_confidence: Option<f32>,
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

    if let Some(relation_types) = options.relation_types {
        let relation_types = relation_types
            .into_iter()
            .map(|item| item.trim().to_string())
            .filter(|item| !item.is_empty())
            .collect::<Vec<_>>();

        if !relation_types.is_empty() {
            args.push("--relation-types".to_string());
            args.push(relation_types.join(","));
        }
    }

    if let Some(min_confidence) = options.min_confidence {
        args.push("--min-confidence".to_string());
        args.push(min_confidence.to_string());
    }

    run_ts_cli("impact", &args)
}
