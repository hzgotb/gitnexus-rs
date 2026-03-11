use std::path::Path;

use anyhow::Result;

use crate::commands::delegate::run_ts_cli;

#[derive(Debug, Clone, Default)]
pub struct WikiOptions {
    pub force: bool,
    pub model: Option<String>,
    pub base_url: Option<String>,
    pub api_key: Option<String>,
    pub concurrency: Option<u16>,
    pub gist: bool,
}

pub fn run(path: Option<&Path>, options: WikiOptions) -> Result<()> {
    let mut args = Vec::<String>::new();

    if let Some(path) = path {
        args.push(path.to_string_lossy().to_string());
    }

    if options.force {
        args.push("--force".to_string());
    }

    if let Some(model) = options.model {
        args.push("--model".to_string());
        args.push(model);
    }

    if let Some(base_url) = options.base_url {
        args.push("--base-url".to_string());
        args.push(base_url);
    }

    if let Some(api_key) = options.api_key {
        args.push("--api-key".to_string());
        args.push(api_key);
    }

    if let Some(concurrency) = options.concurrency {
        args.push("--concurrency".to_string());
        args.push(concurrency.to_string());
    }

    if options.gist {
        args.push("--gist".to_string());
    }

    run_ts_cli("wiki", &args)
}
